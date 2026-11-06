//! Stage 19 ownership checking for class move values.
//!
//! This pass is intentionally backend-independent. It runs after ordinary
//! semantic/type checking and records errors in source vocabulary before MIR
//! lowering or either native backend can observe an invalid ownership graph.

use std::collections::{HashMap, HashSet};

use crate::ast::{self, Argument, AssignOp, BinaryOp, ClassMember, Expr, Item, Stmt};
use crate::builtins::Builtin;
use crate::diagnostics::{Diagnostic, DiagnosticSource, FixApplicability, FixEdit};
use crate::narrowing::{Fact, FactsByUse};
use crate::source::Span;
use crate::symbols::{
    BindingId, BindingKind, BindingOwnership, BindingResolution, BorrowSource, ClosureId,
    ReturnBorrow,
};
use crate::types::{
    CollectionFamily, FunctionInvocationMode, ResolvedType, RetainedLifetime, ReturnBorrowKind,
};

pub mod cleanup;
pub(crate) mod owned_storage;
mod pending;
mod property_hooks;
mod retained;
pub(crate) mod writeback;
pub(crate) use retained::SourceTypes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowAccess {
    Readonly,
    Writable,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BorrowRoot {
    Binding(BindingId),
    Receiver,
    EnclosingEnvironment(ClosureId),
    Temporary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueProvenance {
    Owned,
    BorrowBound(Vec<BorrowRoot>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureAcquisitionKind {
    ReadonlyLease,
    WritableLease,
    CopyIntoEnvironment,
    MoveIntoEnvironment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureAcquisition {
    pub environment_binding_id: BindingId,
    pub source_binding_id: BindingId,
    pub kind: CaptureAcquisitionKind,
    pub source_type: ResolvedType,
    pub roots: Vec<BorrowRoot>,
    pub source_span: Option<Span>,
    pub capture_span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationConsumption {
    Repeatable,
    Once,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureEscapeClassification {
    Local,
    Owned,
    ReturnedBorrow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureOwnershipInfo {
    pub closure_id: ClosureId,
    pub provenance: ValueProvenance,
    pub acquisitions: Vec<CaptureAcquisition>,
    pub release_order: Vec<usize>,
    pub escape: ClosureEscapeClassification,
    pub invocation_consumption: InvocationConsumption,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OwnershipAnalysis {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) closures: HashMap<ClosureId, ClosureOwnershipInfo>,
    /// Final local carrier ownership, keyed by the canonical binding identity.
    /// Name/type checking cannot decide this before returned-loan inference.
    pub(crate) binding_ownership: HashMap<BindingId, BindingOwnership>,
    pub(crate) writable_borrow_sources: HashSet<Span>,
    pub(crate) return_borrows: HashMap<Span, ReturnBorrow>,
    pub(crate) retained_callables: HashMap<Span, RetainedCallableInfo>,
    pub(crate) cleanup: cleanup::Analysis,
    pub(crate) writeback_constraints: Vec<writeback::Constraint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedSource {
    pub source: BorrowSource,
    /// Propagate the input carrier's loans instead of borrowing the input itself.
    pub inherited: bool,
    pub writable: bool,
    pub lifetime: RetainedLifetime,
    pub owned_path: Vec<crate::class_layout::PropertyId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CallbackCaptureSource {
    Binding(BindingId),
    Expression(Span),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackCapturePlace {
    pub source: CallbackCaptureSource,
    /// Innermost call first; consuming this path keeps unrelated callers separate.
    pub callers: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallbackCaptureTarget {
    pub declaration: Span,
    pub captures: Vec<(BindingId, Vec<CallbackCapturePlace>)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetainedCallableInfo {
    pub returns: Vec<RetainedSource>,
    pub constructs: Vec<RetainedSource>,
    pub requires_independent: Vec<BorrowSource>,
    /// Inputs whose incoming dependencies enter an enum payload. Source loans
    /// travel with the enum value; borrow-bound closures cannot.
    pub payload_inputs: Vec<BorrowSource>,
    pub pending_selections: Vec<pending::Selection>,
    pub stored_sources: Vec<owned_storage::Store>,
    pub exceptional_stored_sources: Vec<owned_storage::Store>,
    pub removals: Vec<owned_storage::Selection>,
    pub exceptional_removals: Vec<owned_storage::Selection>,
    pub returned_selectors: Vec<owned_storage::Selector>,
    pub returned_storage: Option<owned_storage::ReturnedStorage>,
    pub(crate) writeback: writeback::Summary,
}

#[derive(Debug, Clone)]
struct Parameter {
    name: String,
    move_type: bool,
    class_type: bool,
    generic: bool,
    take: bool,
    writable: bool,
    borrow: bool,
}

#[derive(Debug, Clone, Default)]
struct Signature {
    declaration: Option<Span>,
    params: Vec<Parameter>,
    returns: Option<String>,
    returns_collection: Option<CollectionInfo>,
    returns_move_type: bool,
    return_borrow: Option<ReturnBorrow>,
    receiver: Option<UseMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReturnedClosureProvenance {
    Owned,
    Retained(ReturnBorrow),
    Invalid,
}

impl Signature {
    fn for_function_value(
        function: &crate::types::SemanticFunctionType<ResolvedType>,
        declaration: Option<Span>,
        move_enums: &MoveEnums,
    ) -> Self {
        Self {
            declaration,
            params: function
                .parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| Parameter {
                    name: index.to_string(),
                    move_type: resolved_type_is_move_type(&parameter.ty, move_enums),
                    class_type: matches!(&parameter.ty, ResolvedType::Class(class) if class.inline_copy_type().is_none()),
                    generic: resolved_type_requires_conservative_move(&parameter.ty),
                    take: parameter.ownership_mode == crate::types::FunctionTypeParameterMode::Take,
                    writable: parameter.ownership_mode == crate::types::FunctionTypeParameterMode::Writable,
                    borrow: false,
                })
                .collect(),
            returns_move_type: (resolved_type_is_move_type(&function.return_type, move_enums)
                || resolved_type_requires_conservative_move(&function.return_type))
                && function
                    .return_borrow
                    .is_none_or(|borrow| borrow.kind == ReturnBorrowKind::Retained),
            return_borrow: function.return_borrow.map(|borrow| {
                let crate::types::FunctionBorrowSource::Parameter(index) = borrow.source;
                ReturnBorrow {
                    kind: borrow.kind,
                    source: BorrowSource::Parameter(index),
                    writable: borrow.writable,
                    lifetime: borrow.lifetime,
                }
            }),
            ..Self::default()
        }
    }

    /// Bind call arguments to parameters by name (decision 0098). Ownership
    /// analysis runs over source (written) order, but a named argument's
    /// ownership mode comes from the parameter it binds to, not its source
    /// position — so both directions of the mapping are needed.
    fn bind_arguments(&self, args: &[Argument]) -> crate::arg_binding::BoundArguments {
        let param_names: Vec<&str> = self
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        // Defaults do not affect the argument<->parameter mapping, only the
        // missing-required diagnostic (owned by semantic analysis), so `false`
        // for every parameter is sufficient here.
        let param_has_default = vec![false; param_names.len()];
        let arg_names: Vec<Option<&str>> = args
            .iter()
            .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
            .collect();
        crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names)
    }
}

fn writable_shared_constructor_signature() -> Signature {
    Signature {
        params: vec![Parameter {
            name: "value".to_string(),
            move_type: true,
            class_type: false,
            generic: true,
            take: true,
            writable: false,
            borrow: false,
        }],
        returns_move_type: true,
        ..Signature::default()
    }
}

pub(crate) fn returned_retained_provenance(
    function: &ast::FunctionDecl,
    closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    binding_resolution: &BindingResolution,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    source_lifetime: &dyn Fn(&ResolvedType) -> Option<RetainedLifetime>,
    constructor_sources: &dyn Fn(&Expr) -> Option<Vec<usize>>,
) -> Option<ReturnedClosureProvenance> {
    // Owned results can transport existing source loans. Callers preserve
    // ordinary Value-borrow inference rather than confusing it with retention.
    struct ClosureSources<'a> {
        values: HashMap<BindingId, Vec<Expr>>,
        resolve_call: std::cell::RefCell<&'a mut dyn FnMut(&Expr) -> ReturnBorrowLookup>,
        source_lifetime: &'a dyn Fn(&ResolvedType) -> Option<RetainedLifetime>,
        constructor_sources: &'a dyn Fn(&Expr) -> Option<Vec<usize>>,
    }

    fn merge(
        current: ReturnedClosureProvenance,
        incoming: ReturnedClosureProvenance,
    ) -> ReturnedClosureProvenance {
        match (current, incoming) {
            (ReturnedClosureProvenance::Invalid, _) | (_, ReturnedClosureProvenance::Invalid) => {
                ReturnedClosureProvenance::Invalid
            }
            (ReturnedClosureProvenance::Owned, ReturnedClosureProvenance::Owned) => {
                ReturnedClosureProvenance::Owned
            }
            (ReturnedClosureProvenance::Owned, ReturnedClosureProvenance::Retained(borrow))
            | (ReturnedClosureProvenance::Retained(borrow), ReturnedClosureProvenance::Owned) => {
                ReturnedClosureProvenance::Retained(borrow)
            }
            (
                ReturnedClosureProvenance::Retained(current),
                ReturnedClosureProvenance::Retained(incoming),
            ) if current.source == incoming.source => {
                ReturnedClosureProvenance::Retained(ReturnBorrow {
                    kind: ReturnBorrowKind::Retained,
                    source: current.source,
                    lifetime: current.lifetime.join(incoming.lifetime),
                    writable: current
                        .kind
                        .join_writable(current.writable, incoming.writable),
                })
            }
            (ReturnedClosureProvenance::Retained(_), ReturnedClosureProvenance::Retained(_)) => {
                ReturnedClosureProvenance::Invalid
            }
        }
    }

    fn record_declaration(
        declaration: &ast::VarDecl,
        binding_resolution: &BindingResolution,
        sources: &mut HashMap<BindingId, Vec<Expr>>,
    ) {
        for binding in &declaration.bindings {
            if let Some(id) = binding_resolution.declaration_by_span.get(&binding.span) {
                sources
                    .entry(*id)
                    .or_default()
                    .push(declaration.initializer.clone());
            }
        }
    }

    fn record_assignment(
        assignment: &ast::Assignment,
        binding_resolution: &BindingResolution,
        sources: &mut HashMap<BindingId, Vec<Expr>>,
    ) {
        if assignment.op != AssignOp::Assign {
            return;
        }
        let Expr::Variable { span, .. } = ungroup_expr(&assignment.target) else {
            return;
        };
        if let Some(id) = binding_resolution.uses_by_span.get(span) {
            sources
                .entry(*id)
                .or_default()
                .push(assignment.value.clone());
        }
    }

    fn collect_sources(
        block: &ast::Block,
        binding_resolution: &BindingResolution,
        sources: &mut HashMap<BindingId, Vec<Expr>>,
    ) {
        for statement in &block.statements {
            match statement {
                Stmt::VarDecl(declaration) => {
                    record_declaration(declaration, binding_resolution, sources)
                }
                Stmt::Assignment(assignment) => {
                    record_assignment(assignment, binding_resolution, sources)
                }
                Stmt::Block(block) => collect_sources(block, binding_resolution, sources),
                Stmt::If(statement) => {
                    if let Some(given) = &statement.given {
                        collect_sources(&given.block, binding_resolution, sources);
                    }
                    collect_sources(&statement.then_block, binding_resolution, sources);
                    if let Some(branch) = &statement.else_branch {
                        match branch {
                            ast::ElseBranch::If(statement) => collect_sources(
                                &ast::Block {
                                    statements: vec![Stmt::If((**statement).clone())],
                                    span: statement.span,
                                },
                                binding_resolution,
                                sources,
                            ),
                            ast::ElseBranch::Block(block) => {
                                collect_sources(block, binding_resolution, sources)
                            }
                        }
                    }
                    if let Some(finally) = &statement.finally {
                        collect_sources(&finally.block, binding_resolution, sources);
                    }
                }
                Stmt::While(statement) => {
                    if let Some(given) = &statement.given {
                        collect_sources(&given.block, binding_resolution, sources);
                    }
                    collect_sources(&statement.body, binding_resolution, sources);
                    if let Some(finally) = &statement.finally {
                        collect_sources(&finally.block, binding_resolution, sources);
                    }
                }
                Stmt::DoWhile(statement) => {
                    collect_sources(&statement.body, binding_resolution, sources);
                    if let Some(finally) = &statement.finally {
                        collect_sources(&finally.block, binding_resolution, sources);
                    }
                }
                Stmt::For(statement) => {
                    if let Some(initializer) = &statement.initializer {
                        match initializer {
                            ast::ForInitializer::VarDecl(declaration) => {
                                record_declaration(declaration, binding_resolution, sources)
                            }
                            ast::ForInitializer::Assignment(assignment) => {
                                record_assignment(assignment, binding_resolution, sources)
                            }
                        }
                    }
                    collect_sources(&statement.body, binding_resolution, sources);
                    if let Some(ast::ForIncrement::Assignment(assignment)) = &statement.increment {
                        record_assignment(assignment, binding_resolution, sources);
                    }
                }
                Stmt::Foreach(statement) => {
                    collect_sources(&statement.body, binding_resolution, sources)
                }
                Stmt::Try(statement) => {
                    collect_sources(&statement.body, binding_resolution, sources);
                    for catch in &statement.catches {
                        collect_sources(&catch.body, binding_resolution, sources);
                    }
                    if let Some(finally) = &statement.finally {
                        collect_sources(&finally.body, binding_resolution, sources);
                    }
                }
                Stmt::Echo { .. }
                | Stmt::Return { .. }
                | Stmt::Throw(_)
                | Stmt::Increment(_)
                | Stmt::Expr { .. }
                | Stmt::Break { .. }
                | Stmt::Continue { .. } => {}
            }
        }
    }

    fn from_expr(
        expr: &Expr,
        function: &ast::FunctionDecl,
        closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
        binding_resolution: &BindingResolution,
        sources: &ClosureSources<'_>,
        visiting: &mut HashSet<BindingId>,
    ) -> Option<ReturnedClosureProvenance> {
        match ungroup_expr(expr) {
            Expr::Closure(closure) => {
                let semantic = closures.get(&ClosureId::from_span(closure.span))?;
                let mut borrow: Option<ReturnBorrow> = None;
                for capture in &semantic.captures {
                    let declaration = binding_resolution
                        .declarations_by_id
                        .get(&capture.source_binding_id)?;
                    let candidate = if capture.mode == ast::ClosureCaptureMode::Take {
                        // Moving a source-carrying value preserves its actual
                        // dependencies, including those needed by its cleanup.
                        if (sources.source_lifetime)(&capture.source_type).is_none() {
                            continue;
                        }
                        match from_binding(
                            capture.source_binding_id,
                            function,
                            closures,
                            binding_resolution,
                            sources,
                            visiting,
                        )? {
                            ReturnedClosureProvenance::Owned => continue,
                            ReturnedClosureProvenance::Retained(borrow) => borrow,
                            ReturnedClosureProvenance::Invalid => {
                                return Some(ReturnedClosureProvenance::Invalid);
                            }
                        }
                    } else {
                        match declaration.kind {
                            BindingKind::MethodReceiver if function.consuming_this => {
                                return Some(ReturnedClosureProvenance::Invalid);
                            }
                            BindingKind::MethodReceiver => ReturnBorrow {
                                kind: ReturnBorrowKind::Retained,
                                source: BorrowSource::Receiver,
                                lifetime: RetainedLifetime::LastUse,
                                writable: capture.mode == ast::ClosureCaptureMode::Writable,
                            },
                            BindingKind::FunctionParameter
                            | BindingKind::MethodParameter
                            | BindingKind::ClosureParameter => {
                                let index = function
                                    .params
                                    .iter()
                                    .position(|parameter| parameter.name == declaration.name)?;
                                ReturnBorrow {
                                    kind: ReturnBorrowKind::Retained,
                                    source: BorrowSource::Parameter(index),
                                    lifetime: RetainedLifetime::LastUse,
                                    writable: capture.mode == ast::ClosureCaptureMode::Writable,
                                }
                            }
                            _ => {
                                let mut borrow = match borrowed_binding_source(
                                    capture.source_binding_id,
                                    function,
                                    closures,
                                    binding_resolution,
                                    sources,
                                    visiting,
                                ) {
                                    ReturnBorrowLookup::Resolved(Some(borrow)) => borrow,
                                    ReturnBorrowLookup::Unresolved => return None,
                                    _ => return Some(ReturnedClosureProvenance::Invalid),
                                };
                                if capture.mode == ast::ClosureCaptureMode::Writable
                                    && !borrow.writable
                                {
                                    return Some(ReturnedClosureProvenance::Invalid);
                                }
                                borrow.kind = ReturnBorrowKind::Retained;
                                borrow.writable = capture.mode == ast::ClosureCaptureMode::Writable;
                                borrow
                            }
                        }
                    };
                    match borrow {
                        Some(existing) if existing.source != candidate.source => {
                            return Some(ReturnedClosureProvenance::Invalid);
                        }
                        Some(existing) => {
                            borrow = Some(ReturnBorrow {
                                lifetime: existing.lifetime.join(candidate.lifetime),
                                writable: existing
                                    .kind
                                    .join_writable(existing.writable, candidate.writable),
                                ..existing
                            });
                        }
                        None => borrow = Some(candidate),
                    }
                }
                Some(borrow.map_or(
                    ReturnedClosureProvenance::Owned,
                    ReturnedClosureProvenance::Retained,
                ))
            }
            Expr::Variable { span, .. } => {
                let id = *binding_resolution.uses_by_span.get(span)?;
                from_binding(
                    id,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                )
            }
            Expr::Null { .. } => Some(ReturnedClosureProvenance::Owned),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => Some(merge(
                from_expr(
                    left,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                )?,
                from_expr(
                    right,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                )?,
            )),
            Expr::New { args, .. } => {
                let indexes = (sources.constructor_sources)(expr)?;
                let mut found = ReturnedClosureProvenance::Owned;
                for index in indexes {
                    let argument = &args.get(index)?.value;
                    let mut borrow = match source_borrow(
                        argument,
                        function,
                        closures,
                        binding_resolution,
                        sources,
                        visiting,
                    ) {
                        ReturnBorrowLookup::Resolved(Some(borrow)) => borrow,
                        ReturnBorrowLookup::Unresolved => return None,
                        _ => {
                            // A constructor borrowing a local/temporary cannot
                            // become a safely escaping owned closure capture.
                            return Some(ReturnedClosureProvenance::Invalid);
                        }
                    };
                    borrow.kind = ReturnBorrowKind::Retained;
                    borrow.writable = false;
                    borrow.lifetime = RetainedLifetime::Cleanup;
                    found = merge(found, ReturnedClosureProvenance::Retained(borrow));
                }
                Some(found)
            }
            Expr::FunctionCall { .. }
            | Expr::MethodCall { .. }
            | Expr::StaticCall { .. }
            | Expr::CallableCall { .. }
            | Expr::PropertyAccess { .. } => {
                let lookup = (sources.resolve_call.borrow_mut())(expr);
                if matches!(lookup, ReturnBorrowLookup::Resolved(None)) {
                    return Some(ReturnedClosureProvenance::Owned);
                }
                match source_borrow(
                    expr,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                ) {
                    ReturnBorrowLookup::Resolved(Some(borrow))
                        if borrow.kind == ReturnBorrowKind::Retained =>
                    {
                        Some(ReturnedClosureProvenance::Retained(borrow))
                    }
                    ReturnBorrowLookup::Resolved(None) => Some(ReturnedClosureProvenance::Invalid),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn borrowed_binding_source(
        id: BindingId,
        function: &ast::FunctionDecl,
        closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
        binding_resolution: &BindingResolution,
        sources: &ClosureSources<'_>,
        visiting: &mut HashSet<BindingId>,
    ) -> ReturnBorrowLookup {
        let Some(values) = sources.values.get(&id) else {
            return ReturnBorrowLookup::Resolved(None);
        };
        if !visiting.insert(id) {
            return ReturnBorrowLookup::Resolved(None);
        }
        // A borrowed binding aliases its initializer's home. Later assignment
        // replaces that home's value without rebinding the alias to a new owner.
        let result = values
            .first()
            .map(|value| {
                source_borrow(
                    value,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                )
            })
            .unwrap_or(ReturnBorrowLookup::Resolved(None));
        visiting.remove(&id);
        match result {
            ReturnBorrowLookup::Resolved(Some(borrow))
                if borrow.kind != ReturnBorrowKind::Value =>
            {
                ReturnBorrowLookup::Resolved(None)
            }
            result => result,
        }
    }

    fn source_borrow(
        expr: &Expr,
        function: &ast::FunctionDecl,
        closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
        binding_resolution: &BindingResolution,
        sources: &ClosureSources<'_>,
        visiting: &mut HashSet<BindingId>,
    ) -> ReturnBorrowLookup {
        let mut unresolved = false;
        let mut resolve = |source: &Expr| {
            if let Expr::Variable { span, .. } = source {
                let Some(id) = binding_resolution.uses_by_span.get(span) else {
                    unresolved = true;
                    return ReturnBorrowLookup::Unresolved;
                };
                let declaration = &binding_resolution.declarations_by_id[id];
                if let Some((index, parameter)) =
                    function.params.iter().enumerate().find(|(_, parameter)| {
                        binding_resolution.declaration_by_span.get(&parameter.span) == Some(id)
                    })
                {
                    if !parameter.take {
                        return ReturnBorrowLookup::Resolved(Some(ReturnBorrow {
                            kind: ReturnBorrowKind::Value,
                            source: BorrowSource::Parameter(index),
                            lifetime: RetainedLifetime::LastUse,
                            writable: parameter.writable,
                        }));
                    }
                }
                if declaration.kind == BindingKind::MethodReceiver && !function.consuming_this {
                    return ReturnBorrowLookup::Resolved(Some(ReturnBorrow {
                        kind: ReturnBorrowKind::Value,
                        source: BorrowSource::Receiver,
                        lifetime: RetainedLifetime::LastUse,
                        writable: function.has_writable_receiver(),
                    }));
                }
                if let ReturnBorrowLookup::Resolved(Some(borrow)) = borrowed_binding_source(
                    *id,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                ) {
                    return ReturnBorrowLookup::Resolved(Some(borrow));
                }
                return match from_binding(
                    *id,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                ) {
                    Some(ReturnedClosureProvenance::Retained(borrow)) => {
                        ReturnBorrowLookup::Resolved(Some(borrow))
                    }
                    Some(_) => ReturnBorrowLookup::Resolved(None),
                    None => {
                        unresolved = true;
                        ReturnBorrowLookup::Unresolved
                    }
                };
            }
            let lookup = (sources.resolve_call.borrow_mut())(source);
            unresolved |= matches!(lookup, ReturnBorrowLookup::Unresolved);
            lookup
        };
        let borrow = expr_return_borrow(expr, function, &mut resolve, &HashSet::new());
        if borrow.is_none() && unresolved {
            ReturnBorrowLookup::Unresolved
        } else {
            ReturnBorrowLookup::Resolved(borrow)
        }
    }

    fn from_binding(
        id: BindingId,
        function: &ast::FunctionDecl,
        closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
        binding_resolution: &BindingResolution,
        sources: &ClosureSources<'_>,
        visiting: &mut HashSet<BindingId>,
    ) -> Option<ReturnedClosureProvenance> {
        let Some(values) = sources.values.get(&id) else {
            let declaration = binding_resolution.declarations_by_id.get(&id)?;
            if let Some(lifetime) = declaration
                .source_type
                .as_ref()
                .and_then(sources.source_lifetime)
                .or_else(|| {
                    (declaration.ownership == BindingOwnership::Owned
                        && declaration
                            .source_type
                            .as_ref()
                            .is_some_and(crate::types::resolved_type_is_symbolic))
                    .then_some(RetainedLifetime::Cleanup)
                })
            {
                if let Some(index) = function.params.iter().position(|parameter| {
                    binding_resolution.declaration_by_span.get(&parameter.span) == Some(&id)
                }) {
                    return Some(ReturnedClosureProvenance::Retained(ReturnBorrow {
                        source: BorrowSource::Parameter(index),
                        kind: ReturnBorrowKind::Retained,
                        lifetime,
                        writable: declaration.source_type.as_ref().is_some_and(|ty| {
                            crate::types::ByteRegionKind::from_resolved(ty)
                                .is_some_and(|region| region.is_writable())
                        }),
                    }));
                }
            }
            return binding_resolution
                .declarations_by_id
                .get(&id)
                .filter(|declaration| declaration.ownership == BindingOwnership::Owned)
                .map(|_| ReturnedClosureProvenance::Owned);
        };
        if !visiting.insert(id) {
            return Some(ReturnedClosureProvenance::Invalid);
        }
        let result = (|| {
            let mut found = None;
            for source in values {
                let candidate = from_expr(
                    source,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    visiting,
                )?;
                found = Some(found.map_or(candidate, |current| merge(current, candidate)));
            }
            found
        })();
        visiting.remove(&id);
        result
    }

    fn visit_block(
        block: &ast::Block,
        function: &ast::FunctionDecl,
        closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
        binding_resolution: &BindingResolution,
        sources: &ClosureSources<'_>,
        found: &mut Option<ReturnedClosureProvenance>,
    ) {
        for statement in &block.statements {
            match statement {
                Stmt::Return {
                    expr: Some(expr), ..
                } => {
                    let Some(candidate) = from_expr(
                        expr,
                        function,
                        closures,
                        binding_resolution,
                        sources,
                        &mut HashSet::new(),
                    ) else {
                        continue;
                    };
                    *found = Some(found.map_or(candidate, |current| merge(current, candidate)));
                }
                Stmt::Block(block) => visit_block(
                    block,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    found,
                ),
                Stmt::If(statement) => {
                    visit_block(
                        &statement.then_block,
                        function,
                        closures,
                        binding_resolution,
                        sources,
                        found,
                    );
                    if let Some(branch) = &statement.else_branch {
                        match branch {
                            ast::ElseBranch::If(statement) => visit_block(
                                &ast::Block {
                                    statements: vec![Stmt::If((**statement).clone())],
                                    span: statement.span,
                                },
                                function,
                                closures,
                                binding_resolution,
                                sources,
                                found,
                            ),
                            ast::ElseBranch::Block(block) => visit_block(
                                block,
                                function,
                                closures,
                                binding_resolution,
                                sources,
                                found,
                            ),
                        }
                    }
                }
                Stmt::While(statement) => visit_block(
                    &statement.body,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    found,
                ),
                Stmt::DoWhile(statement) => visit_block(
                    &statement.body,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    found,
                ),
                Stmt::For(statement) => visit_block(
                    &statement.body,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    found,
                ),
                Stmt::Foreach(statement) => visit_block(
                    &statement.body,
                    function,
                    closures,
                    binding_resolution,
                    sources,
                    found,
                ),
                Stmt::Try(statement) => {
                    visit_block(
                        &statement.body,
                        function,
                        closures,
                        binding_resolution,
                        sources,
                        found,
                    );
                    for catch in &statement.catches {
                        visit_block(
                            &catch.body,
                            function,
                            closures,
                            binding_resolution,
                            sources,
                            found,
                        );
                    }
                    if let Some(finally) = &statement.finally {
                        visit_block(
                            &finally.body,
                            function,
                            closures,
                            binding_resolution,
                            sources,
                            found,
                        );
                    }
                }
                Stmt::VarDecl(_)
                | Stmt::Assignment(_)
                | Stmt::Echo { .. }
                | Stmt::Throw(_)
                | Stmt::Increment(_)
                | Stmt::Expr { .. }
                | Stmt::Return { expr: None, .. }
                | Stmt::Break { .. }
                | Stmt::Continue { .. } => {}
            }
        }
    }

    let mut sources = HashMap::new();
    let body = function.body.as_block()?;
    collect_sources(body, binding_resolution, &mut sources);
    let sources = ClosureSources {
        values: sources,
        resolve_call: std::cell::RefCell::new(resolve_call),
        source_lifetime,
        constructor_sources,
    };
    let mut found = None;
    visit_block(
        body,
        function,
        closures,
        binding_resolution,
        &sources,
        &mut found,
    );
    found
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Borrowed,
    BorrowedOrOwned,
    Owned,
    Given { at: Span },
    MaybeGiven { at: Span },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallExecution {
    Always,
    Never,
    Maybe,
}

#[derive(Debug, Clone, Copy)]
enum FunctionStorageBoundary<'a> {
    /// The constructed value carries its inputs' source loans; their escape is
    /// checked wherever that value goes, as for a constructed class.
    Transport(&'a str),
    Deferred {
        destination: &'a str,
        title: &'a str,
    },
}

#[derive(Debug, Clone)]
struct Binding {
    id: OwnershipSlotId,
    canonical_id: Option<BindingId>,
    class: Option<String>,
    collection: Option<CollectionInfo>,
    mixed: bool,
    borrowed_place: bool,
    borrow_root: Option<BorrowSourceRoot>,
    borrow_parents: Vec<OwnershipSlotId>,
    writeback_homes: Vec<writeback::Home>,
    writable: bool,
    state: State,
    function_type: Option<crate::types::SemanticFunctionType<ResolvedType>>,
    retained_value: Option<RetainedValueState>,
    scope_depth: usize,
}

#[derive(Debug, Clone)]
struct RetainedValueState {
    kind: RetainedValueKind,
    closure_id: Option<ClosureId>,
    provenance: ValueProvenance,
    leases: Vec<RetainedLoan>,
    nonescaping_parameter: bool,
    take_parameter_insertion: Option<Span>,
    storage: owned_storage::Contents,
    enum_payloads: Option<HashMap<(crate::enums::EnumCaseSource, usize), RetainedValueState>>,
}

impl RetainedValueState {
    fn refresh_stored_loans(&mut self) {
        self.leases = self.storage.loans();
        self.provenance = retained::loan_provenance(&self.leases);
    }

    fn retain_cleanup_loans(&mut self) {
        self.leases
            .retain(|loan| loan.lifetime == RetainedLifetime::Cleanup);
        self.storage.retain_cleanup_loans();
        if let Some(payloads) = &mut self.enum_payloads {
            for payload in payloads.values_mut() {
                payload.retain_cleanup_loans();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RetainedValueKind {
    Closure,
    SourceCarrier,
}

#[derive(Debug, Clone)]
struct RetainedLoan {
    root: BorrowRoot,
    root_key: BorrowPlace,
    access: BorrowAccess,
    capture_span: Span,
    source_depth: usize,
    inherited: bool,
    lifetime: RetainedLifetime,
    /// Freeze intermediate carriers without treating their descriptors as storage owners.
    reborrows: Vec<(BorrowPlace, BorrowAccess)>,
    pending_capture: Option<BindingId>,
}

impl RetainedLoan {
    fn same_source(&self, other: &Self) -> bool {
        self.root == other.root
            && self.inherited == other.inherited
            && self.access == other.access
            && self.reborrows == other.reborrows
            && self.pending_capture == other.pending_capture
            && self.root_key == other.root_key
    }

    fn places(&self) -> impl Iterator<Item = (&BorrowPlace, BorrowAccess)> {
        std::iter::once((&self.root_key, self.access))
            .chain(self.reborrows.iter().map(|(root, access)| (root, *access)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BorrowPlace {
    owner: String,
    fields: Vec<crate::class_layout::PropertyId>,
    projectable: bool,
}

impl BorrowPlace {
    fn overlaps(&self, other: &Self) -> bool {
        self.owner == other.owner
            && (self.fields.starts_with(&other.fields) || other.fields.starts_with(&self.fields))
    }

    fn opaque(mut self) -> Self {
        self.projectable = false;
        self
    }

    fn project(mut self, property: crate::class_layout::PropertyId) -> Self {
        if self.projectable {
            self.fields.push(property);
        }
        self
    }

    fn extend(&mut self, fields: &[crate::class_layout::PropertyId]) {
        if self.projectable {
            self.fields.extend_from_slice(fields);
        }
    }
}

impl From<String> for BorrowPlace {
    fn from(owner: String) -> Self {
        Self {
            owner,
            fields: Vec::new(),
            projectable: true,
        }
    }
}

impl From<&str> for BorrowPlace {
    fn from(owner: &str) -> Self {
        owner.to_owned().into()
    }
}

/// An immutable absent value has no owner to keep alive. Keep that fact
/// distinct from an expression whose owning temporary cannot be retained.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BorrowSourceRoot {
    Place(BorrowPlace),
    Alternatives(Vec<BorrowPlace>),
    ImmutableNull,
}

impl BorrowSourceRoot {
    fn places(&self) -> &[BorrowPlace] {
        match self {
            Self::Place(place) => std::slice::from_ref(place),
            Self::Alternatives(places) => places,
            Self::ImmutableNull => &[],
        }
    }

    fn from_places(places: impl IntoIterator<Item = BorrowPlace>) -> Self {
        let mut unique = Vec::new();
        for place in places {
            if !unique.contains(&place) {
                unique.push(place);
            }
        }
        match unique.len() {
            0 => Self::ImmutableNull,
            1 => Self::Place(unique.pop().expect("one source place")),
            _ => Self::Alternatives(unique),
        }
    }

    fn map_places(self, map: impl FnMut(BorrowPlace) -> BorrowPlace) -> Self {
        Self::from_places(self.places().iter().cloned().map(map))
    }

    fn place_key(&self) -> Option<&BorrowPlace> {
        match self {
            Self::Place(key) => Some(key),
            Self::Alternatives(_) | Self::ImmutableNull => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OwnershipSlotId(usize);

#[derive(Debug, Clone)]
struct PropertyInfo {
    class: Option<String>,
    collection: Option<CollectionInfo>,
    mixed: bool,
    move_type: bool,
    writable: bool,
    borrowed_source: Option<crate::types::RetainedFieldAccess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CollectionInfo {
    family: CollectionFamily,
    value_move: bool,
    value_mixed: bool,
    value_class: Option<String>,
    value_collection: Option<Box<CollectionInfo>>,
}

#[derive(Debug, Clone, Default)]
struct Scopes(
    Vec<HashMap<String, Option<Binding>>>,
    cleanup::State,
    Vec<pending::Loan>,
    Vec<owned_storage::Selection>,
);

impl Scopes {
    fn new() -> Self {
        Self(
            vec![HashMap::new()],
            cleanup::State::default(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn push(&mut self) {
        self.0.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.0.pop();
    }

    fn lexical_depth(&self) -> usize {
        self.0.len()
    }

    fn truncate_to(&mut self, lexical_depth: usize) {
        self.0.truncate(lexical_depth);
    }

    fn apply_place_alternatives(
        &mut self,
        mut places: Vec<BorrowPlace>,
        mut apply: impl FnMut(BorrowPlace, &mut Self),
    ) {
        let Some(first) = places.pop() else { return };
        if places.is_empty() {
            apply(first, self);
            return;
        }
        // Exactly one candidate is selected at runtime. A joined removal must
        // preserve every source that survives in another candidate's branch.
        let before = self.clone();
        apply(first, self);
        for place in places {
            let mut branch = before.clone();
            apply(place, &mut branch);
            let previous = self.clone();
            self.merge_from(&previous, &branch);
        }
    }

    fn declare(&mut self, name: String, binding: Binding) {
        self.0
            .last_mut()
            .expect("ownership scope")
            .insert(name, Some(binding));
    }

    fn get(&self, name: &str) -> Option<&Binding> {
        for scope in self.0.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return binding.as_ref();
            }
        }
        None
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Binding> {
        for scope in self.0.iter_mut().rev() {
            if let Some(binding) = scope.get_mut(name) {
                return binding.as_mut();
            }
        }
        None
    }

    fn get_by_slot(&self, id: OwnershipSlotId) -> Option<&Binding> {
        self.0
            .iter()
            .rev()
            .flat_map(|scope| scope.values())
            .flatten()
            .find(|binding| binding.id == id)
    }

    fn borrowed_from(
        &self,
        root: &BorrowPlace,
        through: &[OwnershipSlotId],
        mode: UseMode,
    ) -> Option<&str> {
        let mut ancestors = HashSet::new();
        let mut parents = through.to_vec();
        while let Some(id) = parents.pop() {
            if ancestors.insert(id) {
                if let Some(binding) = self.get_by_slot(id) {
                    parents.extend_from_slice(&binding.borrow_parents);
                }
            }
        }
        self.0.iter().rev().find_map(|scope| {
            scope.iter().find_map(|(name, binding)| {
                binding
                    .as_ref()
                    .is_some_and(|binding| {
                        !ancestors.contains(&binding.id)
                            && (matches!(mode, UseMode::Write | UseMode::Give)
                                || (binding.borrowed_place && binding.writable))
                            && binding.borrow_root.as_ref().is_some_and(|source| {
                                source
                                    .places()
                                    .iter()
                                    .any(|borrowed| borrowed.overlaps(root))
                            })
                    })
                    .then_some(name.as_str())
            })
        })
    }

    fn get_by_canonical(&self, id: BindingId) -> Option<(&str, &Binding)> {
        self.0.iter().rev().find_map(|scope| {
            scope.iter().find_map(|(name, binding)| {
                binding
                    .as_ref()
                    .filter(|binding| binding.canonical_id == Some(id))
                    .map(|binding| (name.as_str(), binding))
            })
        })
    }

    fn get_mut_by_canonical(&mut self, id: BindingId) -> Option<&mut Binding> {
        self.0.iter_mut().rev().find_map(|scope| {
            scope.values_mut().find_map(|binding| {
                binding
                    .as_mut()
                    .filter(|binding| binding.canonical_id == Some(id))
            })
        })
    }

    fn active_retained_loans(&self) -> impl Iterator<Item = (RetainedValueKind, &RetainedLoan)> {
        self.0.iter().flat_map(|scope| {
            scope
                .values()
                .filter_map(Option::as_ref)
                .flat_map(|binding| {
                    let available = matches!(
                        binding.state,
                        State::Owned
                            | State::Borrowed
                            | State::BorrowedOrOwned
                            | State::MaybeGiven { .. }
                    );
                    binding
                        .retained_value
                        .as_ref()
                        .filter(move |_| available)
                        .into_iter()
                        .flat_map(|value| value.leases.iter().map(|loan| (value.kind, loan)))
                })
        })
    }

    fn conflicting_retained_loan(
        &self,
        mut conflicts: impl FnMut(&RetainedLoan) -> bool,
    ) -> Option<(RetainedValueKind, &RetainedLoan)> {
        // Hashed binding order must not choose a diagnostic's source or category.
        self.active_retained_loans()
            .filter(|(_, loan)| conflicts(loan))
            .min_by_key(|(kind, loan)| {
                (
                    loan.capture_span,
                    *kind,
                    loan.access == BorrowAccess::Writable,
                )
            })
    }

    fn release_unused_retained_loans(
        &mut self,
        remaining: &[Stmt],
        current_scope_only: bool,
        live_bindings: Option<HashSet<BindingId>>,
    ) {
        let mut required = live_bindings;
        if let Some(required) = &mut required {
            // A live borrowed carrier keeps its source carrier's dependencies
            // live too, even when that source is never invoked directly again.
            loop {
                let before = required.len();
                for binding in self.0.iter().flat_map(|scope| scope.values()).flatten() {
                    if matches!(binding.state, State::Given { .. }) {
                        continue;
                    }
                    let live = binding.canonical_id.is_none_or(|id| required.contains(&id));
                    for loan in binding
                        .retained_value
                        .iter()
                        .flat_map(|value| &value.leases)
                    {
                        if live || loan.lifetime == RetainedLifetime::Cleanup {
                            if let BorrowRoot::Binding(id) = loan.root {
                                required.insert(id);
                            }
                        }
                    }
                }
                if before == required.len() {
                    break;
                }
            }
        }
        let start = if current_scope_only && required.is_none() {
            self.0.len().saturating_sub(1)
        } else {
            0
        };
        for scope in &mut self.0[start..] {
            for (name, binding) in scope {
                let Some(binding) = binding else {
                    continue;
                };
                // Invocation-only capture loans and regions end at last use.
                // Owned captured carriers may still observe their sources
                // during ordinary cleanup, so preserve those dependencies.
                if binding.retained_value.is_some()
                    && required.as_ref().map_or_else(
                        || !statements_use_variable(remaining, name),
                        |required| {
                            binding
                                .canonical_id
                                .is_some_and(|id| !required.contains(&id))
                        },
                    )
                {
                    if let Some(value) = &mut binding.retained_value {
                        value.retain_cleanup_loans();
                    }
                }
            }
        }
    }

    fn release_unused_borrows(&mut self, remaining: &[Stmt], current_scope_only: bool) {
        let start = if current_scope_only {
            self.0.len().saturating_sub(1)
        } else {
            0
        };
        for scope in &mut self.0[start..] {
            for (name, binding) in scope {
                if let Some(binding) = binding {
                    if binding.borrow_root.is_some() && !statements_use_variable(remaining, name) {
                        binding.borrow_root = None;
                    }
                }
            }
        }
    }

    fn merge_from(&mut self, left: &Self, right: &Self) {
        self.1.merge_from(&left.1, &right.1);
        self.2 = left.2.clone();
        pending::append_loans(&mut self.2, &right.2);
        self.3 = left
            .3
            .iter()
            .filter(|selection| right.3.contains(selection))
            .cloned()
            .collect();
        for (index, scope) in self.0.iter_mut().enumerate() {
            for (name, binding) in scope {
                let Some(binding) = binding else {
                    continue;
                };
                let Some(left_state) = left
                    .0
                    .get(index)
                    .and_then(|scope| scope.get(name))
                    .and_then(Option::as_ref)
                else {
                    continue;
                };
                let Some(right_state) = right
                    .0
                    .get(index)
                    .and_then(|scope| scope.get(name))
                    .and_then(Option::as_ref)
                else {
                    continue;
                };
                binding.state = join_state(&left_state.state, &right_state.state);
                binding.writeback_homes = left_state.writeback_homes.clone();
                writeback::append(&mut binding.writeback_homes, &right_state.writeback_homes);
                binding.retained_value = join_retained_value(
                    left_state.retained_value.as_ref(),
                    right_state.retained_value.as_ref(),
                );
            }
        }
    }
}

fn join_retained_value(
    left: Option<&RetainedValueState>,
    right: Option<&RetainedValueState>,
) -> Option<RetainedValueState> {
    match (left, right) {
        (Some(left), Some(right)) if left.closure_id == right.closure_id => {
            let mut joined = left.clone();
            joined.join_enum_payloads(right);
            joined
                .storage
                .merge_from(&right.storage, &joined.leases, &right.leases);
            retained::append_loans(&mut joined.leases, &right.leases);
            joined.provenance = joined_provenance(&left.provenance, &right.provenance);
            Some(joined)
        }
        (Some(left), Some(right)) => {
            let mut joined = left.clone();
            joined.join_enum_payloads(right);
            joined.closure_id = None;
            joined
                .storage
                .merge_from(&right.storage, &joined.leases, &right.leases);
            retained::append_loans(&mut joined.leases, &right.leases);
            joined.provenance = joined_provenance(&left.provenance, &right.provenance);
            joined.nonescaping_parameter =
                left.nonescaping_parameter || right.nonescaping_parameter;
            Some(joined)
        }
        (Some(left), None) | (None, Some(left)) => Some(left.clone()),
        (None, None) => None,
    }
}

impl RetainedValueState {
    fn project_enum_payload(
        &self,
        case: crate::enums::EnumCaseSource,
        field: usize,
        opaque_kind: Option<RetainedValueKind>,
    ) -> Option<Self> {
        if let Some(fields) = &self.enum_payloads {
            return fields.get(&(case, field)).cloned();
        }
        // A call or branch join can retain dependencies without proving which
        // payload carries them. A source-carrying projection must keep them.
        let kind = opaque_kind?;
        if self.leases.is_empty() {
            return None;
        }
        let mut projected = retained::iterator_value(self.leases.clone());
        projected.kind = kind;
        Some(projected)
    }

    fn join_enum_payloads(&mut self, incoming: &Self) {
        let (Some(fields), Some(incoming)) = (&mut self.enum_payloads, &incoming.enum_payloads)
        else {
            self.enum_payloads = None;
            return;
        };
        for (field, value) in incoming {
            let joined = join_retained_value(fields.get(field), Some(value))
                .expect("incoming payload supplies a retained value");
            fields.insert(*field, joined);
        }
    }
}

fn provenance_roots(provenance: &ValueProvenance) -> Vec<BorrowRoot> {
    match provenance {
        ValueProvenance::Owned => Vec::new(),
        ValueProvenance::BorrowBound(roots) => roots.clone(),
    }
}

fn joined_provenance(left: &ValueProvenance, right: &ValueProvenance) -> ValueProvenance {
    let mut roots = provenance_roots(left);
    roots.extend(provenance_roots(right));
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        ValueProvenance::Owned
    } else {
        ValueProvenance::BorrowBound(roots)
    }
}

fn join_state(left: &State, right: &State) -> State {
    match (left, right) {
        (State::Borrowed, State::Borrowed) => State::Borrowed,
        (State::BorrowedOrOwned, State::Borrowed)
        | (State::Borrowed, State::BorrowedOrOwned)
        | (State::BorrowedOrOwned, State::BorrowedOrOwned)
        | (State::BorrowedOrOwned, State::Owned)
        | (State::Owned, State::BorrowedOrOwned)
        | (State::Borrowed, State::Owned)
        | (State::Owned, State::Borrowed) => State::BorrowedOrOwned,
        (State::Owned, State::Owned) => State::Owned,
        (State::Given { at: left }, State::Given { at: right }) if left == right => {
            State::Given { at: *left }
        }
        (State::Given { at }, State::Given { .. })
        | (State::MaybeGiven { at }, _)
        | (_, State::MaybeGiven { at })
        | (State::Owned, State::Given { at })
        | (State::Given { at }, State::Owned)
        | (State::Borrowed, State::Given { at })
        | (State::Given { at }, State::Borrowed)
        | (State::BorrowedOrOwned, State::Given { at })
        | (State::Given { at }, State::BorrowedOrOwned) => State::MaybeGiven { at: *at },
    }
}

pub fn check_program(program: &ast::Program) -> Vec<Diagnostic> {
    let flow_facts = crate::narrowing::analyze_program(program);
    let inferred_move_returns = HashSet::new();
    let return_borrows = HashMap::new();
    let resolved_types = HashMap::new();
    let move_enum_names = MoveEnums::default();
    let given_preludes = HashMap::new();
    let checked_effect_sites = HashMap::new();
    let catch_error_types = HashMap::new();
    let catch_coverage = HashMap::new();
    let binding_resolution = BindingResolution::default();
    let closures = HashMap::new();
    let callable_value_calls = HashMap::new();
    let assertion_callable_invocations = HashMap::new();
    let list_algorithm_calls = HashMap::new();
    let call_targets = HashMap::new();
    let property_accessor_calls = HashMap::new();
    let property_writes = HashMap::new();
    check_program_with_inferred_move_returns(
        program,
        &OwnershipAnalysisContext {
            inferred_move_returns: &inferred_move_returns,
            return_borrows: &return_borrows,
            specialized_return_borrows: &HashMap::new(),
            resolved_types: &resolved_types,
            interface_conversion_types: &HashMap::new(),
            flow_facts: &flow_facts,
            move_enum_names: &move_enum_names,
            given_preludes: &given_preludes,
            checked_effect_sites: &checked_effect_sites,
            catch_error_types: &catch_error_types,
            catch_coverage: &catch_coverage,
            binding_resolution: &binding_resolution,
            closures: &closures,
            callable_value_calls: &callable_value_calls,
            callback_capture_targets: &HashMap::new(),
            callback_value_targets: &HashMap::new(),
            assertion_callable_invocations: &assertion_callable_invocations,
            list_algorithm_calls: &list_algorithm_calls,
            call_targets: &call_targets,
            property_accessor_calls: &property_accessor_calls,
            property_writes: &property_writes,
            matches: &HashMap::new(),
            enum_semantics: &[],
            enum_case_constructions: &HashMap::new(),
            contracts: &crate::semantics::contracts::ContractFacts::default(),
            classes: &[],
            generic_call_specializations: &HashMap::new(),
            composition: &crate::trait_composition::CompositionPlan::default(),
            source_types: &SourceTypes::default(),
            retained_callable_seed: &HashMap::new(),
        },
    )
    .diagnostics
}

#[derive(Clone, Copy)]
pub(crate) struct OwnershipAnalysisContext<'a> {
    pub(crate) inferred_move_returns: &'a HashSet<Span>,
    pub(crate) return_borrows: &'a HashMap<Span, Option<ReturnBorrow>>,
    pub(crate) specialized_return_borrows: &'a crate::semantics::SpecializedReturnBorrows,
    pub(crate) resolved_types: &'a HashMap<Span, crate::types::ResolvedType>,
    pub(crate) interface_conversion_types: &'a HashMap<Span, crate::types::ResolvedType>,
    pub(crate) flow_facts: &'a FactsByUse,
    pub(crate) move_enum_names: &'a MoveEnums,
    pub(crate) given_preludes: &'a HashMap<Span, crate::semantics::GivenSemanticInfo>,
    pub(crate) checked_effect_sites: &'a crate::checked_effects::EffectSiteMap,
    pub(crate) catch_error_types: &'a crate::checked_effects::CatchTypeMap,
    pub(crate) catch_coverage: &'a crate::checked_effects::CatchCoverageMap,
    pub(crate) binding_resolution: &'a BindingResolution,
    pub(crate) closures: &'a HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    pub(crate) callable_value_calls: &'a HashMap<Span, crate::semantics::CallableValueCallInfo>,
    pub(crate) callback_capture_targets: &'a HashMap<Span, Vec<CallbackCaptureTarget>>,
    pub(crate) callback_value_targets: &'a HashMap<Span, Vec<CallbackCaptureTarget>>,
    pub(crate) assertion_callable_invocations: &'a HashMap<Span, FunctionInvocationMode>,
    pub(crate) list_algorithm_calls: &'a HashMap<Span, crate::semantics::ListAlgorithmCallInfo>,
    pub(crate) call_targets: &'a HashMap<Span, crate::semantics::CallableTarget>,
    pub(crate) property_accessor_calls: &'a HashMap<Span, crate::semantics::PropertyAccessorCalls>,
    pub(crate) property_writes: &'a HashMap<Span, crate::semantics::PropertyWriteSemanticInfo>,
    pub(crate) matches: &'a HashMap<Span, crate::semantics::MatchSemanticInfo>,
    pub(crate) enum_semantics: &'a [crate::semantics::EnumSemanticInfo],
    pub(crate) enum_case_constructions: &'a HashMap<Span, crate::enums::EnumCaseSource>,
    pub(crate) contracts: &'a crate::semantics::contracts::ContractFacts,
    pub(crate) classes: &'a [crate::semantics::ClassSemanticInfo],
    pub(crate) generic_call_specializations:
        &'a HashMap<Span, crate::semantics::GenericSpecialization>,
    pub(crate) composition: &'a crate::trait_composition::CompositionPlan,
    pub(crate) source_types: &'a SourceTypes,
    pub(crate) retained_callable_seed: &'a HashMap<Span, RetainedCallableInfo>,
}

#[derive(Clone, Copy)]
enum BodySelection {
    Executable,
    Authored,
    Traits,
    Callable(Span),
}

struct MemberOwner<'a> {
    name: &'a String,
    type_params: &'a [ast::TypeParamDecl],
    members: &'a [ClassMember],
    context: crate::property_hooks::PropertyHookContext,
}

fn member_owner(item: &Item) -> Option<MemberOwner<'_>> {
    use crate::property_hooks::PropertyHookContext;
    match item {
        Item::Class(class) => Some(MemberOwner {
            name: &class.name,
            type_params: &class.type_params,
            members: &class.members,
            context: PropertyHookContext::Class,
        }),
        Item::Trait(declaration) => Some(MemberOwner {
            name: &declaration.name,
            type_params: &declaration.type_params,
            members: &declaration.members,
            context: PropertyHookContext::Trait,
        }),
        _ => None,
    }
}

fn program_callables(
    program: &ast::Program,
) -> Vec<(std::borrow::Cow<'_, ast::FunctionDecl>, Option<&str>)> {
    let mut callables = Vec::new();
    for item in &program.items {
        if let Item::Function(function) = item {
            callables.push((std::borrow::Cow::Borrowed(function), None));
        } else if let Some(owner) = member_owner(item) {
            callables.extend(owner.members.iter().flat_map(|member| {
                crate::property_hooks::member_callables(member, owner.context)
                    .map(|function| (function, Some(owner.name.as_str())))
            }));
        }
    }
    callables
}

pub(crate) fn check_program_with_inferred_move_returns(
    program: &ast::Program,
    context: &OwnershipAnalysisContext<'_>,
) -> OwnershipAnalysis {
    let mut analysis = check_selected_program(program, context, BodySelection::Executable);
    if program
        .items
        .iter()
        .any(|item| matches!(item, Item::Trait(_)))
    {
        // Source-only traits contribute diagnostics, never executable cleanup
        // or closure facts for an uninstantiated body.
        let traits = check_selected_program(program, context, BodySelection::Traits);
        analysis.diagnostics.extend(traits.diagnostics);
    }
    analysis
}

pub(crate) fn check_authored_program_with_inferred_move_returns(
    program: &ast::Program,
    context: &OwnershipAnalysisContext<'_>,
) -> OwnershipAnalysis {
    check_selected_program(program, context, BodySelection::Authored)
}

pub(crate) fn check_callable_with_inferred_move_returns(
    program: &ast::Program,
    context: &OwnershipAnalysisContext<'_>,
    declaration: Span,
) -> OwnershipAnalysis {
    check_selected_program(program, context, BodySelection::Callable(declaration))
}

fn check_selected_program(
    program: &ast::Program,
    context: &OwnershipAnalysisContext<'_>,
    selection: BodySelection,
) -> OwnershipAnalysis {
    let declaration = match selection {
        BodySelection::Callable(declaration) => Some(declaration),
        _ => None,
    };
    let OwnershipAnalysisContext {
        inferred_move_returns,
        return_borrows,
        specialized_return_borrows,
        resolved_types,
        interface_conversion_types,
        flow_facts,
        move_enum_names,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        catch_coverage,
        binding_resolution,
        closures,
        callable_value_calls,
        callback_capture_targets,
        callback_value_targets,
        assertion_callable_invocations,
        list_algorithm_calls,
        call_targets,
        property_accessor_calls,
        property_writes,
        matches,
        enum_semantics,
        enum_case_constructions,
        contracts,
        classes: class_semantics,
        generic_call_specializations,
        composition,
        source_types,
        retained_callable_seed,
    } = *context;
    let classes = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class.name.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let move_nominal_names = move_enum_names
        .declarations()
        .cloned()
        .chain(
            contracts
                .interface_specializations
                .iter()
                .filter(|interface| interface.valid)
                .map(|interface| interface.specialization.name.clone()),
        )
        .collect::<HashSet<_>>();
    let receiver_type_params = program
        .items
        .iter()
        .filter_map(member_owner)
        .map(|owner| (owner.name.clone(), owner.type_params.to_vec()))
        .collect::<HashMap<_, _>>();
    let mut signatures = HashMap::new();
    let mut constructors = HashMap::new();
    let mut methods = HashMap::new();
    let mut enum_cases = HashMap::new();
    let mut properties = HashMap::new();
    let mut static_properties = HashMap::new();

    let mut retained_analysis =
        retained::Analysis::new(class_semantics, contracts, move_enum_names, source_types);
    retained_analysis.seed_callables(retained_callable_seed, composition);
    let early_constructors = program
        .items
        .iter()
        .filter_map(|item| {
            let Item::Class(class) = item else {
                return None;
            };
            let constructor = class.members.iter().find_map(|member| match member {
                ClassMember::Method(method) if method.name == "__construct" => Some(method),
                _ => None,
            });
            Some((
                class.name.clone(),
                constructor.map(|constructor| {
                    signature(
                        constructor,
                        &classes,
                        &move_nominal_names,
                        inferred_move_returns,
                        return_borrows,
                        Some(&class.name),
                        &class.type_params,
                    )
                }),
            ))
        })
        .collect::<HashMap<_, _>>();
    let constructor_sources = |expr: &Expr| {
        let Expr::New { args, .. } = expr else {
            return None;
        };
        let ResolvedType::Class(class) = resolved_types.get(&expr.span())? else {
            return None;
        };
        let Some(constructor) = early_constructors.get(&class.name)? else {
            return Some(Vec::new());
        };
        let bound = constructor.bind_arguments(args);
        constructor
            .params
            .iter()
            .enumerate()
            .filter(|(_, parameter)| parameter.borrow)
            .map(|(index, _)| bound.param_to_arg.get(index).copied().flatten())
            .collect::<Option<Vec<_>>>()
    };
    let source_lifetime = |ty: &ResolvedType| retained_analysis.source_lifetime(ty);

    let mut resolved_return_borrows = return_borrows.clone();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                let mut function_signature = signature(
                    function,
                    &classes,
                    &move_nominal_names,
                    inferred_move_returns,
                    return_borrows,
                    None,
                    &[],
                );
                if let Some(ReturnedClosureProvenance::Retained(borrow)) =
                    returned_retained_provenance(
                        function,
                        closures,
                        binding_resolution,
                        &mut |_| ReturnBorrowLookup::Unresolved,
                        &source_lifetime,
                        &constructor_sources,
                    )
                    .filter(|_| {
                        function_signature
                            .return_borrow
                            .is_none_or(|borrow| borrow.kind != ReturnBorrowKind::Value)
                            || function
                                .return_type
                                .as_ref()
                                .is_some_and(|ty| ty.function.is_some())
                    })
                {
                    function_signature.return_borrow = Some(borrow);
                    function_signature.returns_move_type = true;
                    resolved_return_borrows.insert(function.span, Some(borrow));
                }
                signatures.insert(function.name.clone(), function_signature);
            }
            Item::Class(_) | Item::Trait(_) => {
                let mut class = member_owner(item).unwrap();
                if matches!(
                    class.context,
                    crate::property_hooks::PropertyHookContext::Class
                ) {
                    if let Some(effective) = composition.class(class.name) {
                        class.members = &effective.members;
                    }
                }
                let imported = if matches!(
                    class.context,
                    crate::property_hooks::PropertyHookContext::Trait
                ) {
                    composition
                        .source_trait_members
                        .get(class.name)
                        .map_or(&[][..], Vec::as_slice)
                } else {
                    &[]
                };
                let mut counts = HashMap::new();
                for member in class.members.iter().chain(imported) {
                    *counts
                        .entry(crate::trait_composition::member_name(member))
                        .or_insert(0) += 1;
                }
                for member in class
                    .members
                    .iter()
                    .chain(imported)
                    .filter(|member| counts[crate::trait_composition::member_name(member)] == 1)
                {
                    match member {
                        ClassMember::Property(property) if property.is_static => {
                            static_properties.insert(
                                (class.name.clone(), property.name.clone()),
                                property.writable,
                            );
                        }
                        ClassMember::Property(property) => {
                            let property_class = type_ref_class_name(
                                &property.ty,
                                &classes,
                                Some(class.name.as_str()),
                            );
                            let move_type = type_ref_is_move_type_with_nominals(
                                &property.ty,
                                &classes,
                                &move_nominal_names,
                                Some(class.name.as_str()),
                            ) || type_ref_has_ownership_parameter(
                                &property.ty,
                                class.type_params,
                            );
                            properties.insert(
                                (class.name.clone(), property.name.clone()),
                                PropertyInfo {
                                    class: property_class,
                                    collection: type_ref_collection_info(
                                        &property.ty,
                                        &classes,
                                        &move_nominal_names,
                                        Some(class.name.as_str()),
                                        class.type_params,
                                        &[],
                                    ),
                                    mixed: property.ty.name == "mixed",
                                    move_type,
                                    writable: property.writable,
                                    borrowed_source: None,
                                },
                            );
                        }
                        ClassMember::Constant(_)
                        | ClassMember::Uses(_)
                        | ClassMember::Method(_) => {}
                    }
                    for callable in crate::property_hooks::member_callables(member, class.context) {
                        let mut method_signature = signature(
                            &callable,
                            &classes,
                            &move_nominal_names,
                            inferred_move_returns,
                            return_borrows,
                            Some(class.name.as_str()),
                            class.type_params,
                        );
                        if let Some(ReturnedClosureProvenance::Retained(borrow)) =
                            returned_retained_provenance(
                                &callable,
                                closures,
                                binding_resolution,
                                &mut |_| ReturnBorrowLookup::Unresolved,
                                &source_lifetime,
                                &constructor_sources,
                            )
                            .filter(|_| {
                                method_signature
                                    .return_borrow
                                    .is_none_or(|borrow| borrow.kind != ReturnBorrowKind::Value)
                                    || callable
                                        .return_type
                                        .as_ref()
                                        .is_some_and(|ty| ty.function.is_some())
                            })
                        {
                            method_signature.return_borrow = Some(borrow);
                            method_signature.returns_move_type = true;
                            resolved_return_borrows.insert(callable.span, Some(borrow));
                        }
                        methods.insert(
                            (class.name.clone(), callable.name.clone()),
                            method_signature.clone(),
                        );
                        if callable.name == "__construct" {
                            constructors.insert(class.name.clone(), method_signature);
                            for param in &callable.params {
                                let property_class = type_ref_class_name(
                                    &param.ty,
                                    &classes,
                                    Some(class.name.as_str()),
                                );
                                let move_type = type_ref_is_move_type_with_nominals(
                                    &param.ty,
                                    &classes,
                                    &move_nominal_names,
                                    Some(class.name.as_str()),
                                ) || type_ref_has_ownership_parameter(
                                    &param.ty,
                                    class.type_params,
                                );
                                if param.constructor_role.is_promoted() {
                                    properties.insert(
                                        (class.name.clone(), param.name.clone()),
                                        PropertyInfo {
                                            class: property_class,
                                            collection: type_ref_collection_info(
                                                &param.ty,
                                                &classes,
                                                &move_nominal_names,
                                                Some(class.name.as_str()),
                                                class.type_params,
                                                &[],
                                            ),
                                            mixed: param.ty.name == "mixed",
                                            move_type,
                                            writable: param.writable && param.borrow_span.is_none(),
                                            borrowed_source:
                                                crate::types::RetainedFieldAccess::promoted(
                                                    param.borrow_span.is_some(),
                                                    param.writable,
                                                ),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Item::Enum(declaration) => {
                for case in &declaration.cases {
                    if case.payload.is_empty() {
                        continue;
                    }
                    enum_cases.insert(
                        (declaration.name.clone(), case.name.clone()),
                        Signature {
                            params: case
                                .payload
                                .iter()
                                .map(|field| {
                                    let generic = type_ref_has_ownership_parameter(
                                        &field.ty,
                                        &declaration.type_params,
                                    );
                                    let move_type = type_ref_is_move_type_with_nominals(
                                        &field.ty,
                                        &classes,
                                        &move_nominal_names,
                                        None,
                                    ) || generic;
                                    Parameter {
                                        name: field.name.clone(),
                                        move_type,
                                        class_type: type_ref_class_name(&field.ty, &classes, None)
                                            .is_some(),
                                        generic,
                                        take: move_type,
                                        writable: false,
                                        borrow: false,
                                    }
                                })
                                .collect(),
                            returns_move_type: move_enum_names
                                .declaration_is_move(&declaration.name),
                            ..Signature::default()
                        },
                    );
                }
            }
            Item::Interface(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    retained_analysis.add_template_fields(&properties);

    let mut checker = Checker {
        classes,
        receiver_type_params,
        signatures,
        constructors,
        methods,
        enum_cases,
        properties,
        static_properties,
        inferred_move_returns: inferred_move_returns.clone(),
        return_borrows: resolved_return_borrows,
        specialized_return_borrows,
        resolved_types,
        interface_conversion_types,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        catch_coverage,
        exception_scopes: Vec::new(),
        move_enum_names: move_enum_names.clone(),
        move_nominal_names,
        receiver_class: None,
        receiver_writable: false,
        current_type_params: Vec::new(),
        current_return_borrow: None,
        active_assignment_writes: HashSet::new(),
        active_assignment_targets: HashSet::new(),
        active_borrows: Vec::new(),
        when_result_modes: Vec::new(),
        conditional_borrow_sources: HashMap::new(),
        flow_facts,
        binding_resolution,
        closures,
        callable_value_calls,
        callback_capture_targets,
        callback_value_targets,
        assertion_callable_invocations,
        list_algorithm_calls,
        call_targets,
        property_accessor_calls,
        property_writes,
        matches,
        enum_semantics,
        enum_case_constructions,
        retained: retained_analysis,
        composition,
        writeback: writeback::Analysis::new(program, class_semantics, composition),
        generic_call_specializations,
        next_binding_id: 0,
        diagnostics: Vec::new(),
        closure_ownership: HashMap::new(),
        binding_ownership: HashMap::new(),
        writable_borrow_sources: HashSet::new(),
        closure_values: HashMap::new(),
        analyzed_closures: HashSet::new(),
        prepared_closure_evaluations: HashSet::new(),
        cleanup_owner: None,
        cleanup_analysis: cleanup::Analysis::default(),
        cleanup_yield_regions: Vec::new(),
    };
    checker.infer_retained_callables(program);
    let mut top_level_scopes = Scopes::new();
    let mut top_level_falls_through = true;
    for item in &program.items {
        if matches!(selection, BodySelection::Traits) && !matches!(item, Item::Trait(_)) {
            continue;
        }
        if matches!(selection, BodySelection::Executable) && matches!(item, Item::Trait(_)) {
            continue;
        }
        if declaration.is_some_and(|selected| match item {
            Item::Function(function) => function.span != selected,
            // Composed methods retain authored source identities. Select them
            // from the class members below, not from the class's source range.
            Item::Class(_) | Item::Trait(_) => false,
            _ => true,
        }) {
            continue;
        }
        match item {
            Item::Function(function) => checker.check_function(function, None),
            Item::Class(_) | Item::Trait(_) => {
                let class = member_owner(item).unwrap();
                for member in class.members {
                    match member {
                        ClassMember::Property(property) if declaration.is_none() => {
                            if let Some(initializer) = &property.initializer {
                                let cleanup_context = checker.cleanup_begin_callable(property.span);
                                let previous_receiver =
                                    checker.receiver_class.replace(class.name.clone());
                                let mut scopes = Scopes::new();
                                let retained_value =
                                    checker.prepare_retained_value(initializer, &mut scopes);
                                let function_storage_valid =
                                    retained_value.as_ref().is_none_or(|value| {
                                        if property.is_static {
                                            checker.reject_deferred_function_storage(
                                                value,
                                                initializer.span(),
                                                "static property storage",
                                                "Static Function-Value Storage Is Not Yet Available",
                                            )
                                        } else if matches!(
                                            ungroup_expr(initializer),
                                            Expr::Closure(closure)
                                                if closure.captures.as_ref().is_some_and(|clause| !clause.captures.is_empty())
                                        ) {
                                            let diagnostic = Diagnostic::new(
                                                "E0658",
                                                "instance property initializer cannot capture values",
                                                initializer.span(),
                                            )
                                            .with_title(
                                                "Capturing Property Initializer Is Not Allowed",
                                            )
                                            .with_help(
                                                "use a no-capture closure initializer or move a fully owned closure into the property during construction",
                                            );
                                            checker.diagnostics.push(
                                                checker.with_function_value_cause(
                                                    diagnostic,
                                                    value,
                                                ),
                                            );
                                            false
                                        } else {
                                            checker.validate_owned_function_storage(
                                                value,
                                                initializer.span(),
                                                "an instance property",
                                            )
                                        }
                                    });
                                if type_ref_is_move_type_with_nominals(
                                    &property.ty,
                                    &checker.classes,
                                    &checker.move_nominal_names,
                                    Some(class.name.as_str()),
                                ) || type_ref_has_ownership_parameter(
                                    &property.ty,
                                    class.type_params,
                                ) {
                                    checker.reject_borrowed_result(
                                        initializer,
                                        &scopes,
                                        "borrowed result cannot initialize an owning property",
                                        "initialize the property with an independently owned value",
                                    );
                                }
                                checker.use_expr(
                                    initializer,
                                    &mut scopes,
                                    if function_storage_valid {
                                        UseMode::Give
                                    } else {
                                        UseMode::Read
                                    },
                                );
                                // Stored initialization transfers only its result;
                                // argument and receiver temporaries still expire here.
                                checker.cleanup_take_expr(initializer, &mut scopes);
                                let mut flow = Flow::fallthrough();
                                checker.cleanup_end_callable(
                                    cleanup_context,
                                    &mut scopes,
                                    &mut flow,
                                    property.span,
                                );
                                checker.receiver_class = previous_receiver;
                            }
                        }
                        ClassMember::Constant(_)
                        | ClassMember::Uses(_)
                        | ClassMember::Property(_)
                        | ClassMember::Method(_) => {}
                    }
                    for callable in crate::property_hooks::member_callables(member, class.context) {
                        if declaration.is_none_or(|selected| callable.span == selected) {
                            checker.check_function(&callable, Some(class.name.as_str()));
                        }
                    }
                }
            }
            Item::Enum(_) | Item::Interface(_) | Item::Constant(_) => {}
            Item::Statement(statement) => {
                if top_level_falls_through {
                    top_level_falls_through = checker
                        .check_statement(statement, &mut top_level_scopes, false)
                        .falls_through;
                }
            }
        }
    }
    OwnershipAnalysis {
        diagnostics: checker.diagnostics,
        closures: checker.closure_ownership,
        binding_ownership: checker.binding_ownership,
        writable_borrow_sources: checker.writable_borrow_sources,
        return_borrows: checker
            .return_borrows
            .into_iter()
            .filter_map(|(span, borrow)| borrow.map(|borrow| (span, borrow)))
            .collect(),
        retained_callables: checker.retained.callables,
        cleanup: checker.cleanup_analysis,
        writeback_constraints: checker.writeback.constraints,
    }
}

/// An unresolved property may still be a stored-field projection. A resolved
/// owned getter must not fall back to borrowing that field's receiver.
#[derive(Clone, Copy)]
pub(crate) enum ReturnBorrowLookup {
    Unresolved,
    /// A checked conversion creates a fresh carrier at this expression.
    OwnedValue,
    Resolved(Option<ReturnBorrow>),
    /// A selected callee's parameter has already been bound to source-order
    /// arguments by the shared argument binder (including named arguments).
    ResolvedArgument {
        borrow: ReturnBorrow,
        argument_index: usize,
    },
}

pub(crate) fn return_borrow_parameters(
    parameters: impl Iterator<Item = (bool, bool)>,
) -> Vec<usize> {
    let borrowed = parameters
        .enumerate()
        .filter(|(_, (borrowed, _))| *borrowed)
        .collect::<Vec<_>>();
    // Symbolic parameters retain source provenance until substitution decides
    // whether the result is Copy or Move (Decision 0105).
    borrowed
        .iter()
        .filter(|(_, (_, symbolic))| borrowed.len() == 1 || *symbolic)
        .map(|(index, _)| *index)
        .collect()
}

/// Adapt closures for the shared return analysis without changing binding keys.
pub(crate) fn closure_borrow_function(
    closure: &ast::ClosureExpression,
    type_params: Vec<ast::TypeParamDecl>,
) -> ast::FunctionDecl {
    let body = match &closure.body {
        ast::ClosureBody::Expression { expression, .. } => ast::Block {
            statements: vec![Stmt::Return {
                expr: Some((**expression).clone()),
                span: expression.span(),
            }],
            span: closure.span,
        },
        ast::ClosureBody::Block(block) => block.clone(),
    };
    let params = closure
        .parameters
        .iter()
        .map(|parameter| ast::Param {
            borrow_span: None,
            constructor_role: ast::ConstructorParameterRole::Ordinary,
            role_and_mode_prefix_span: parameter.type_span,
            take: parameter.take,
            take_span: parameter.take_span,
            writable: parameter.writable,
            writable_span: parameter.writable_span,
            ownership_modifier_insert: parameter.type_span,
            ty: parameter.ty.clone(),
            type_span: parameter.type_span,
            name: parameter.name.clone(),
            name_span: parameter.name_span,
            default: None,
            default_span: None,
            // Closure bindings use the parameter name span as their identity.
            span: parameter.name_span,
        })
        .collect();
    ast::FunctionDecl {
        access: ast::MemberAccess::External,
        access_span: None,
        is_open: false,
        open_span: None,
        is_override: false,
        override_span: None,
        writable_this: false,
        writable_span: None,
        consuming_this: false,
        consuming_span: None,
        is_static: true,
        static_span: None,
        name: "<closure>".to_string(),
        name_span: closure.keyword_span,
        type_params,
        params,
        return_type: closure
            .return_type
            .as_ref()
            .map(|return_type| return_type.ty.clone()),
        throws: None,
        availability: None,
        body: ast::FunctionBody::Block(body),
        syntax: Box::new(ast::FunctionSyntax::synthetic(closure.span)),
        modifier_prefix_span: closure.span,
        span: closure.span,
    }
}

/// After body checking, infer through the canonical binding identities and
/// control-flow graph. The earlier signature pass has neither local binding
/// facts nor checked call targets and remains deliberately conservative.
pub(crate) fn function_return_borrow_with_bindings(
    function: &ast::FunctionDecl,
    borrowed_parameters: Vec<usize>,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    bindings: &BindingResolution,
    graph: &crate::control_flow::ControlFlowGraph,
) -> Option<ReturnBorrow> {
    infer_return_borrows_with_bindings(function, borrowed_parameters, resolve_call, bindings, graph)
        .returned
}

struct ReturnBorrowInference {
    returned: Option<ReturnBorrow>,
    conditionals: HashMap<Span, BorrowOrigins>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BorrowOrigin {
    Receiver,
    Parameter(usize),
    Local(BindingId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BorrowOrigins {
    sources: Vec<BorrowOrigin>,
    kind: ReturnBorrowKind,
    lifetime: RetainedLifetime,
    writable: bool,
    borrowed: bool,
}

impl BorrowOrigins {
    fn contract(&self) -> Option<ReturnBorrow> {
        if !self.borrowed {
            return None;
        }
        let source = match self.sources.as_slice() {
            [BorrowOrigin::Receiver] => BorrowSource::Receiver,
            [BorrowOrigin::Parameter(index)] => BorrowSource::Parameter(*index),
            _ => return None,
        };
        Some(ReturnBorrow {
            source,
            kind: self.kind,
            lifetime: self.lifetime,
            writable: self.writable,
        })
    }

    fn forwarded(mut self, returned: ReturnBorrow) -> Self {
        self.writable = returned
            .kind
            .compose_writable(self.kind, self.writable, returned.writable);
        self.kind = returned.kind;
        self.lifetime = self.lifetime.join(returned.lifetime);
        self.borrowed = true;
        self
    }
}

struct ConditionalBorrowSources {
    origins: BorrowOrigins,
    bindings: Vec<Option<BindingId>>,
}

fn infer_return_borrows_with_bindings(
    function: &ast::FunctionDecl,
    borrowed_parameters: Vec<usize>,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    bindings: &BindingResolution,
    graph: &crate::control_flow::ControlFlowGraph,
) -> ReturnBorrowInference {
    use crate::control_flow::NodeAction;
    let mut entry = HashMap::new();
    for index in borrowed_parameters {
        let parameter = &function.params[index];
        if let Some(id) = bindings.declaration_by_span.get(&parameter.span) {
            entry.insert(
                ReturnBorrowPlace::Binding(*id),
                ReturnBorrowAlias::Places(BorrowOrigins {
                    kind: ReturnBorrowKind::Value,
                    sources: vec![BorrowOrigin::Parameter(index)],
                    lifetime: RetainedLifetime::LastUse,
                    writable: parameter.writable,
                    borrowed: true,
                }),
            );
        }
    }
    for parameter in &function.params {
        if let Some(id) = bindings.declaration_by_span.get(&parameter.span) {
            if let Some(binding) = bindings.declarations_by_id.get(id) {
                entry
                    .entry(ReturnBorrowPlace::Binding(*id))
                    .or_insert_with(|| ReturnBorrowAlias::local(binding));
            }
        }
    }
    let analysis = ReturnBorrowAliases {
        function,
        bindings,
        entry,
        resolve_call: std::cell::RefCell::new(resolve_call),
        value_yields: &graph.value_yields,
        match_bindings: &graph.match_bindings,
    };
    let solved = crate::dataflow::solve_forward(graph, &analysis);
    let mut returned = ReturnBorrowAlias::Null;
    for node in &graph.nodes {
        let Some(state) = &solved.inputs[node.id.0] else {
            continue;
        };
        if let NodeAction::Statement(Stmt::Return { expr, .. }) = &node.action {
            returned = returned.join(expr.as_ref().map_or(ReturnBorrowAlias::Unknown, |expr| {
                analysis.expression(expr, state)
            }));
        }
    }
    let returned = match returned {
        ReturnBorrowAlias::Places(origins) => origins.contract(),
        ReturnBorrowAlias::Null | ReturnBorrowAlias::Unknown => None,
    };
    let conditionals = graph
        .value_results
        .iter()
        .filter_map(|(span, node)| {
            let state = solved.inputs[node.0].as_ref()?;
            match state.get(&ReturnBorrowPlace::Conditional(*span))? {
                ReturnBorrowAlias::Places(origins) => Some((*span, origins.clone())),
                _ => None,
            }
        })
        .collect();
    ReturnBorrowInference {
        returned,
        conditionals,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReturnBorrowAlias {
    // Unknown includes fresh temporaries without a stable local owner.
    Unknown,
    Null,
    Places(BorrowOrigins),
}

impl ReturnBorrowAlias {
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::Null, value) | (value, Self::Null) => value,
            (Self::Places(mut left), Self::Places(right)) if left.kind == right.kind => {
                left.sources.extend(right.sources);
                left.sources.sort();
                left.sources.dedup();
                left.writable = left.kind.join_writable(left.writable, right.writable);
                left.lifetime = left.lifetime.join(right.lifetime);
                left.borrowed |= right.borrowed;
                Self::Places(left)
            }
            _ => Self::Unknown,
        }
    }

    fn local(binding: &crate::symbols::BindingDeclaration) -> Self {
        Self::Places(BorrowOrigins {
            sources: vec![BorrowOrigin::Local(binding.id)],
            kind: ReturnBorrowKind::Value,
            lifetime: RetainedLifetime::LastUse,
            writable: binding.writable,
            borrowed: false,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ReturnBorrowPlace {
    Binding(BindingId),
    Conditional(Span),
}

struct ReturnBorrowAliases<'a> {
    function: &'a ast::FunctionDecl,
    bindings: &'a BindingResolution,
    entry: HashMap<ReturnBorrowPlace, ReturnBorrowAlias>,
    resolve_call: std::cell::RefCell<&'a mut dyn FnMut(&Expr) -> ReturnBorrowLookup>,
    value_yields: &'a HashMap<crate::control_flow::NodeId, (Span, Expr)>,
    match_bindings:
        &'a HashMap<crate::control_flow::NodeId, crate::control_flow::MatchBindingSource>,
}

impl ReturnBorrowAliases<'_> {
    fn expression(
        &self,
        expr: &Expr,
        state: &HashMap<ReturnBorrowPlace, ReturnBorrowAlias>,
    ) -> ReturnBorrowAlias {
        let lookup = (self.resolve_call.borrow_mut())(expr);
        if matches!(lookup, ReturnBorrowLookup::OwnedValue) {
            return ReturnBorrowAlias::Unknown;
        }
        match ungroup_expr(expr) {
            Expr::Null { .. } => ReturnBorrowAlias::Null,
            Expr::This { .. } if !self.function.is_static && !self.function.consuming_this => {
                ReturnBorrowAlias::Places(BorrowOrigins {
                    sources: vec![BorrowOrigin::Receiver],
                    kind: ReturnBorrowKind::Value,
                    lifetime: RetainedLifetime::LastUse,
                    writable: self.function.has_writable_receiver(),
                    borrowed: true,
                })
            }
            Expr::Variable { span, .. } => self
                .bindings
                .uses_by_span
                .get(span)
                .and_then(|id| state.get(&ReturnBorrowPlace::Binding(*id)))
                .cloned()
                .unwrap_or(ReturnBorrowAlias::Unknown),
            Expr::When(_) | Expr::Match { .. } => state
                .get(&ReturnBorrowPlace::Conditional(ungroup_expr(expr).span()))
                .cloned()
                .unwrap_or(ReturnBorrowAlias::Unknown),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => self
                .expression(left, state)
                .join(self.expression(right, state)),
            Expr::PropertyAccess { object, .. }
                if matches!(lookup, ReturnBorrowLookup::Unresolved) =>
            {
                self.projected(object, state)
            }
            Expr::Index { collection, .. } => self.projected(collection, state),
            Expr::PropertyAccess { object, .. } => self.call(Some(object), &[], lookup, state),
            Expr::FunctionCall { args, .. }
            | Expr::StaticCall { args, .. }
            | Expr::CallableCall { args, .. } => self.call(None, args, lookup, state),
            Expr::MethodCall { object, args, .. } => self.call(Some(object), args, lookup, state),
            _ => ReturnBorrowAlias::Unknown,
        }
    }

    fn projected(
        &self,
        source: &Expr,
        state: &HashMap<ReturnBorrowPlace, ReturnBorrowAlias>,
    ) -> ReturnBorrowAlias {
        match self.expression(source, state) {
            ReturnBorrowAlias::Places(mut origins) => {
                origins.writable = false;
                origins.borrowed = true;
                ReturnBorrowAlias::Places(origins)
            }
            other => other,
        }
    }

    fn call(
        &self,
        receiver: Option<&Expr>,
        args: &[Argument],
        lookup: ReturnBorrowLookup,
        state: &HashMap<ReturnBorrowPlace, ReturnBorrowAlias>,
    ) -> ReturnBorrowAlias {
        let (returned, source) = match lookup {
            ReturnBorrowLookup::Resolved(Some(returned)) => {
                let source = match returned.source {
                    BorrowSource::Receiver => receiver,
                    BorrowSource::Parameter(index) => {
                        argument_bound_to_parameter(self.function, args, index)
                            .map(|argument| &argument.value)
                    }
                };
                (returned, source)
            }
            ReturnBorrowLookup::ResolvedArgument {
                borrow,
                argument_index,
            } => (
                borrow,
                args.get(argument_index).map(|argument| &argument.value),
            ),
            _ => return ReturnBorrowAlias::Unknown,
        };
        match source.map(|source| self.expression(source, state)) {
            Some(ReturnBorrowAlias::Places(origins)) => {
                ReturnBorrowAlias::Places(origins.forwarded(returned))
            }
            Some(ReturnBorrowAlias::Null) => ReturnBorrowAlias::Null,
            _ => ReturnBorrowAlias::Unknown,
        }
    }

    fn stored(&self, id: BindingId, value: &ReturnBorrowAlias) -> ReturnBorrowAlias {
        match value {
            ReturnBorrowAlias::Null => ReturnBorrowAlias::Null,
            ReturnBorrowAlias::Places(origins) if origins.borrowed => value.clone(),
            _ => self
                .bindings
                .declarations_by_id
                .get(&id)
                .map(ReturnBorrowAlias::local)
                .unwrap_or(ReturnBorrowAlias::Unknown),
        }
    }

    fn declare(
        &self,
        declaration: &ast::VarDecl,
        state: &mut HashMap<ReturnBorrowPlace, ReturnBorrowAlias>,
    ) {
        let value = self.expression(&declaration.initializer, state);
        for binding in &declaration.bindings {
            if let Some(id) = self.bindings.declaration_by_span.get(&binding.span) {
                state.insert(ReturnBorrowPlace::Binding(*id), self.stored(*id, &value));
            }
        }
    }

    fn assign(
        &self,
        assignment: &ast::Assignment,
        state: &mut HashMap<ReturnBorrowPlace, ReturnBorrowAlias>,
    ) {
        let Expr::Variable { span, .. } = ungroup_expr(&assignment.target) else {
            return;
        };
        let Some(id) = self.bindings.uses_by_span.get(span) else {
            return;
        };
        let value = if assignment.op == AssignOp::Assign {
            self.expression(&assignment.value, state)
        } else {
            ReturnBorrowAlias::Unknown
        };
        state.insert(ReturnBorrowPlace::Binding(*id), self.stored(*id, &value));
    }
}

impl crate::dataflow::ForwardAnalysis for ReturnBorrowAliases<'_> {
    // None is unreachable, distinct from a reachable value with unknown roots.
    type State = Option<HashMap<ReturnBorrowPlace, ReturnBorrowAlias>>;

    fn bottom(&self) -> Self::State {
        None
    }
    fn entry_state(&self) -> Self::State {
        Some(self.entry.clone())
    }

    fn transfer(&self, node: &crate::control_flow::Node, input: &Self::State) -> Self::State {
        use crate::control_flow::NodeAction;
        let mut output = input.clone()?;
        if let Some(pattern) = self.match_bindings.get(&node.id) {
            let value = match self.expression(&pattern.scrutinee, &output) {
                ReturnBorrowAlias::Places(mut borrow) if pattern.borrowed => {
                    borrow.writable = false;
                    borrow.borrowed = true;
                    ReturnBorrowAlias::Places(borrow)
                }
                _ => ReturnBorrowAlias::Unknown,
            };
            for span in &pattern.bindings {
                if let Some(id) = self.bindings.declaration_by_span.get(span) {
                    let borrowed =
                        self.bindings
                            .declarations_by_id
                            .get(id)
                            .is_some_and(|binding| {
                                matches!(
                                    binding.ownership,
                                    BindingOwnership::ReadonlyBorrow
                                        | BindingOwnership::WritableBorrow
                                )
                            });
                    output.insert(
                        ReturnBorrowPlace::Binding(*id),
                        if borrowed {
                            value.clone()
                        } else {
                            self.stored(*id, &ReturnBorrowAlias::Unknown)
                        },
                    );
                }
            }
        }
        match &node.action {
            NodeAction::Statement(Stmt::VarDecl(declaration))
            | NodeAction::ForInitializer(ast::ForInitializer::VarDecl(declaration)) => {
                self.declare(declaration, &mut output);
            }
            NodeAction::Statement(Stmt::Assignment(assignment))
            | NodeAction::ForInitializer(ast::ForInitializer::Assignment(assignment)) => {
                self.assign(assignment, &mut output);
            }
            NodeAction::ForIncrement(ast::ForIncrement::Assignment(assignment)) => {
                self.assign(assignment, &mut output);
            }
            _ => {}
        }
        if let Some((result, expression)) = self.value_yields.get(&node.id) {
            let value = self.expression(expression, &output);
            output.insert(ReturnBorrowPlace::Conditional(*result), value);
        }
        Some(output)
    }

    fn join(&self, state: &mut Self::State, incoming: &Self::State) -> bool {
        let Some(incoming) = incoming else {
            return false;
        };
        let Some(current) = state else {
            *state = Some(incoming.clone());
            return true;
        };
        let mut merged = current.clone();
        for id in current.keys().chain(incoming.keys()) {
            let left = current
                .get(id)
                .cloned()
                .unwrap_or(ReturnBorrowAlias::Unknown);
            let right = incoming
                .get(id)
                .cloned()
                .unwrap_or(ReturnBorrowAlias::Unknown);
            merged.insert(*id, left.join(right));
        }
        if *current == merged {
            return false;
        }
        *current = merged;
        true
    }
}

pub(crate) fn function_return_borrow_with_calls(
    function: &ast::FunctionDecl,
    borrowed_parameters: Vec<usize>,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
) -> Option<ReturnBorrow> {
    if let ast::FunctionBody::Intrinsic { implementation, .. } = function.body {
        return implementation.return_borrow();
    }
    let mut resolve_source = |expr: &Expr| {
        if let Expr::Variable { name, .. } = expr {
            return ReturnBorrowLookup::Resolved(borrowed_parameters.iter().find_map(|&index| {
                let parameter = &function.params[index];
                (parameter.name == *name).then_some(ReturnBorrow {
                    kind: ReturnBorrowKind::Value,
                    source: BorrowSource::Parameter(index),
                    lifetime: RetainedLifetime::LastUse,
                    writable: parameter.writable,
                })
            }));
        }
        resolve_call(expr)
    };
    let mut borrow = None;
    if block_return_borrow(
        function.body.as_block()?,
        function,
        &mut resolve_source,
        &HashSet::new(),
        &mut borrow,
    )
    .is_some()
    {
        borrow
    } else {
        None
    }
}

fn block_return_borrow(
    block: &ast::Block,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    inherited_shadowed: &HashSet<String>,
    borrow: &mut Option<ReturnBorrow>,
) -> Option<bool> {
    let mut shadowed = inherited_shadowed.clone();
    let mut falls_through = true;
    for statement in &block.statements {
        if !falls_through {
            break;
        }
        falls_through =
            statement_return_borrow(statement, function, resolve_call, &mut shadowed, borrow)?;
    }
    Some(falls_through)
}

fn statement_return_borrow(
    statement: &Stmt,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    shadowed: &mut HashSet<String>,
    borrow: &mut Option<ReturnBorrow>,
) -> Option<bool> {
    match statement {
        Stmt::Block(block) => block_return_borrow(block, function, resolve_call, shadowed, borrow),
        Stmt::Return {
            expr: Some(expr), ..
        } => {
            if matches!(ungroup_expr(expr), Expr::Null { .. }) {
                return Some(false);
            }
            let candidate = expr_return_borrow(expr, function, resolve_call, shadowed)?;
            match borrow {
                Some(existing)
                    if existing.source != candidate.source || existing.kind != candidate.kind =>
                {
                    None
                }
                Some(existing) => {
                    existing.writable = existing
                        .kind
                        .join_writable(existing.writable, candidate.writable);
                    existing.lifetime = existing.lifetime.join(candidate.lifetime);
                    Some(false)
                }
                slot @ None => {
                    *slot = Some(candidate);
                    Some(false)
                }
            }
        }
        Stmt::Return { expr: None, .. } => None,
        Stmt::Throw(_) => Some(false),
        Stmt::Try(_) => None,
        Stmt::If(statement) => match constant_bool(&statement.condition) {
            Some(true) => block_return_borrow(
                &statement.then_block,
                function,
                resolve_call,
                shadowed,
                borrow,
            ),
            Some(false) => statement.else_branch.as_ref().map_or(Some(true), |branch| {
                else_branch_return_borrow(branch, function, resolve_call, shadowed, borrow)
            }),
            None => {
                let then_falls = block_return_borrow(
                    &statement.then_block,
                    function,
                    resolve_call,
                    shadowed,
                    borrow,
                )?;
                let else_falls = statement
                    .else_branch
                    .as_ref()
                    .map_or(Some(true), |branch| {
                        else_branch_return_borrow(branch, function, resolve_call, shadowed, borrow)
                    })?;
                Some(then_falls || else_falls)
            }
        },
        Stmt::While(statement) => {
            if constant_bool(&statement.condition) != Some(false) {
                block_return_borrow(&statement.body, function, resolve_call, shadowed, borrow)?;
            }
            Some(crate::return_analysis::statement_falls_through(
                &Stmt::While(statement.clone()),
            ))
        }
        Stmt::DoWhile(statement) => {
            block_return_borrow(&statement.body, function, resolve_call, shadowed, borrow)?;
            Some(crate::return_analysis::statement_falls_through(
                &Stmt::DoWhile(statement.clone()),
            ))
        }
        Stmt::For(statement) => {
            let mut loop_shadowed = shadowed.clone();
            if let Some(ast::ForInitializer::VarDecl(decl)) = &statement.initializer {
                loop_shadowed.extend(decl.bindings.iter().map(|binding| binding.name.clone()));
            }
            if statement
                .condition
                .as_ref()
                .is_none_or(|condition| constant_bool(condition) != Some(false))
            {
                block_return_borrow(
                    &statement.body,
                    function,
                    resolve_call,
                    &loop_shadowed,
                    borrow,
                )?;
            }
            Some(crate::return_analysis::statement_falls_through(&Stmt::For(
                statement.clone(),
            )))
        }
        Stmt::Foreach(statement) => {
            let mut loop_shadowed = shadowed.clone();
            if let Some(first_binding) = &statement.first_binding {
                loop_shadowed.insert(first_binding.name.clone());
            }
            loop_shadowed.insert(statement.value_binding.name.clone());
            block_return_borrow(
                &statement.body,
                function,
                resolve_call,
                &loop_shadowed,
                borrow,
            )?;
            Some(crate::return_analysis::statement_falls_through(
                &Stmt::Foreach(statement.clone()),
            ))
        }
        Stmt::VarDecl(decl) => {
            shadowed.extend(decl.bindings.iter().map(|binding| binding.name.clone()));
            Some(true)
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => Some(false),
        Stmt::Expr { expr, .. } if is_panic_expr(expr) => Some(false),
        Stmt::Assignment(_) | Stmt::Echo { .. } | Stmt::Increment(_) | Stmt::Expr { .. } => {
            Some(true)
        }
    }
}

fn else_branch_return_borrow(
    branch: &ast::ElseBranch,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    shadowed: &HashSet<String>,
    borrow: &mut Option<ReturnBorrow>,
) -> Option<bool> {
    match branch {
        ast::ElseBranch::If(statement) => {
            let mut branch_shadowed = shadowed.clone();
            statement_return_borrow(
                &Stmt::If((**statement).clone()),
                function,
                resolve_call,
                &mut branch_shadowed,
                borrow,
            )
        }
        ast::ElseBranch::Block(block) => {
            block_return_borrow(block, function, resolve_call, shadowed, borrow)
        }
    }
}

fn expr_return_borrow(
    expr: &Expr,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    if matches!(resolve_call(expr), ReturnBorrowLookup::OwnedValue) {
        return None;
    }
    match expr {
        Expr::This { .. } if !function.is_static && !function.consuming_this => {
            Some(ReturnBorrow {
                kind: ReturnBorrowKind::Value,
                source: BorrowSource::Receiver,
                lifetime: RetainedLifetime::LastUse,
                writable: function.has_writable_receiver(),
            })
        }
        Expr::Variable { name, .. } if !shadowed.contains(name) => match resolve_call(expr) {
            ReturnBorrowLookup::Resolved(borrow) => borrow,
            ReturnBorrowLookup::Unresolved
            | ReturnBorrowLookup::OwnedValue
            | ReturnBorrowLookup::ResolvedArgument { .. } => None,
        },
        Expr::Grouped { expr, .. } => expr_return_borrow(expr, function, resolve_call, shadowed),
        Expr::When(_) | Expr::Match { .. } => match resolve_call(expr) {
            ReturnBorrowLookup::Resolved(borrow) => borrow,
            _ => None,
        },
        Expr::Binary {
            left,
            op: BinaryOp::Coalesce,
            right,
            ..
        } => coalesced_return_borrow(left, right, function, resolve_call, shadowed),
        Expr::PropertyAccess { object, .. } => {
            if let ReturnBorrowLookup::Resolved(borrow) = resolve_call(expr) {
                let borrow = borrow?;
                return returned_borrow_source(
                    borrow,
                    Some(object),
                    &[],
                    function,
                    resolve_call,
                    shadowed,
                );
            }
            expr_return_borrow(object, function, resolve_call, shadowed).map(|borrow| {
                ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            })
        }
        Expr::Index { collection, .. } => {
            expr_return_borrow(collection, function, resolve_call, shadowed).map(|borrow| {
                ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            })
        }
        Expr::FunctionCall { args, .. }
        | Expr::StaticCall { args, .. }
        | Expr::CallableCall { args, .. } => {
            returned_call_borrow(expr, None, args, function, resolve_call, shadowed)
        }
        Expr::MethodCall { object, args, .. } => {
            returned_call_borrow(expr, Some(object), args, function, resolve_call, shadowed)
        }
        _ => None,
    }
}

fn coalesced_return_borrow(
    left: &Expr,
    right: &Expr,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    let left_null = matches!(ungroup_expr(left), Expr::Null { .. });
    let right_null = matches!(ungroup_expr(right), Expr::Null { .. });
    let left = (!left_null)
        .then(|| expr_return_borrow(left, function, resolve_call, shadowed))
        .flatten();
    let right = (!right_null)
        .then(|| expr_return_borrow(right, function, resolve_call, shadowed))
        .flatten();

    match (left, right, left_null, right_null) {
        (Some(borrow), None, _, true) | (None, Some(borrow), true, _) => Some(borrow),
        (Some(mut left), Some(right), _, _)
            if left.source == right.source && left.kind == right.kind =>
        {
            left.writable = left.kind.join_writable(left.writable, right.writable);
            left.lifetime = left.lifetime.join(right.lifetime);
            Some(left)
        }
        _ => None,
    }
}

fn ungroup_expr(mut expr: &Expr) -> &Expr {
    while let Expr::Grouped { expr: inner, .. } = expr {
        expr = inner;
    }
    expr
}

fn returned_call_borrow(
    call: &Expr,
    receiver: Option<&Expr>,
    args: &[Argument],
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    match resolve_call(call) {
        ReturnBorrowLookup::Resolved(Some(returned)) => {
            returned_borrow_source(returned, receiver, args, function, resolve_call, shadowed)
        }
        ReturnBorrowLookup::ResolvedArgument {
            borrow: returned,
            argument_index,
        } => expr_return_borrow(
            &args.get(argument_index)?.value,
            function,
            resolve_call,
            shadowed,
        )
        .map(|borrow| ReturnBorrow {
            kind: returned.kind,
            writable: returned_source_access(borrow, returned),
            lifetime: borrow.lifetime.join(returned.lifetime),
            ..borrow
        }),
        ReturnBorrowLookup::Unresolved
        | ReturnBorrowLookup::OwnedValue
        | ReturnBorrowLookup::Resolved(None) => None,
    }
}

fn returned_borrow_source(
    returned: ReturnBorrow,
    receiver: Option<&Expr>,
    args: &[Argument],
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> ReturnBorrowLookup,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    let source = match returned.source {
        BorrowSource::Receiver => receiver?,
        BorrowSource::Parameter(index) => {
            &argument_bound_to_parameter(function, args, index)?.value
        }
    };
    expr_return_borrow(source, function, resolve_call, shadowed).map(|mut borrow| {
        borrow.writable = returned_source_access(borrow, returned);
        borrow.lifetime = borrow.lifetime.join(returned.lifetime);
        borrow.kind = returned.kind;
        borrow
    })
}

fn returned_source_access(source: ReturnBorrow, returned: ReturnBorrow) -> bool {
    returned
        .kind
        .compose_writable(source.kind, source.writable, returned.writable)
}

/// Resolve the source-order argument bound to parameter `param_index` of
/// `function` under named-argument binding (decision 0098). A borrow annotation
/// names a parameter position; the argument feeding it may sit elsewhere in the
/// written call once named arguments reorder.
fn argument_bound_to_parameter<'a>(
    function: &ast::FunctionDecl,
    args: &'a [Argument],
    param_index: usize,
) -> Option<&'a Argument> {
    let param_names: Vec<&str> = function
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    let param_has_default = vec![false; param_names.len()];
    let arg_names: Vec<Option<&str>> = args
        .iter()
        .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
        .collect();
    let bound = crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names);
    bound
        .param_to_arg
        .get(param_index)
        .copied()
        .flatten()
        .and_then(|arg_index| args.get(arg_index))
}

fn signature(
    function: &ast::FunctionDecl,
    classes: &HashSet<String>,
    move_nominal_names: &HashSet<String>,
    inferred_move_returns: &HashSet<Span>,
    return_borrows: &HashMap<Span, Option<ReturnBorrow>>,
    receiver_class: Option<&str>,
    enclosing_type_params: &[ast::TypeParamDecl],
) -> Signature {
    let return_borrow = return_borrows
        .get(&function.span)
        .copied()
        .unwrap_or_else(|| {
            function_return_borrow_with_calls(
                function,
                return_borrow_parameters(function.params.iter().map(|param| {
                    let symbolic = type_ref_has_any_ownership_parameter(
                        &param.ty,
                        &function.type_params,
                        enclosing_type_params,
                    );
                    (
                        (!param.take
                            && (type_ref_is_move_type_with_nominals(
                                &param.ty,
                                classes,
                                move_nominal_names,
                                receiver_class,
                            ) || symbolic))
                            || crate::types::ByteRegionKind::from_source_name(&param.ty.name)
                                .is_some(),
                        symbolic,
                    )
                })),
                &mut |_| ReturnBorrowLookup::Unresolved,
            )
        })
        .filter(|_| {
            function.return_type.as_ref().is_some_and(|ty| {
                type_ref_is_move_type_with_nominals(ty, classes, move_nominal_names, receiver_class)
                    || crate::types::ByteRegionKind::from_source_name(&ty.name).is_some()
                    || type_ref_has_any_ownership_parameter(
                        ty,
                        &function.type_params,
                        enclosing_type_params,
                    )
            })
        })
        .map(|borrow| {
            function
                .return_type
                .as_ref()
                .and_then(|ty| crate::types::ByteRegionKind::from_source_name(&ty.name))
                .map_or(borrow, |kind| ReturnBorrow {
                    source: borrow.source,
                    kind: ReturnBorrowKind::Retained,
                    lifetime: RetainedLifetime::LastUse,
                    writable: kind.is_writable(),
                })
        });
    Signature {
        declaration: Some(function.span),
        params: function
            .params
            .iter()
            .map(|param| Parameter {
                name: param.name.clone(),
                move_type: type_ref_is_move_type_with_nominals(
                    &param.ty,
                    classes,
                    move_nominal_names,
                    receiver_class,
                ),
                class_type: type_ref_class_name(&param.ty, classes, receiver_class).is_some(),
                generic: type_ref_has_any_ownership_parameter(
                    &param.ty,
                    &function.type_params,
                    enclosing_type_params,
                ),
                take: param.take,
                writable: param.writable,
                borrow: param.borrow_span.is_some(),
            })
            .collect(),
        returns: function
            .return_type
            .as_ref()
            .and_then(|ty| type_ref_class_name(ty, classes, receiver_class)),
        returns_collection: function.return_type.as_ref().and_then(|ty| {
            type_ref_collection_info(
                ty,
                classes,
                move_nominal_names,
                receiver_class,
                &function.type_params,
                enclosing_type_params,
            )
        }),
        returns_move_type: function.return_type.as_ref().is_some_and(|ty| {
            (type_ref_is_move_type_with_nominals(ty, classes, move_nominal_names, receiver_class)
                || type_ref_has_any_ownership_parameter(
                    ty,
                    &function.type_params,
                    enclosing_type_params,
                ))
                && return_borrow.is_none_or(|borrow| borrow.kind == ReturnBorrowKind::Retained)
        }) || (function.return_type.is_none()
            && inferred_move_returns.contains(&function.span)),
        return_borrow,
        receiver: receiver_class.map(|_| {
            if function.consuming_this {
                UseMode::Give
            } else if function.has_writable_receiver() {
                UseMode::Write
            } else {
                UseMode::Read
            }
        }),
    }
}

fn type_ref_has_ownership_parameter(
    ty: &crate::types::TypeRef,
    params: &[ast::TypeParamDecl],
) -> bool {
    if crate::inline_value::InlineValueKind::for_nominal(&ty.name, ty.arguments.len()).is_some() {
        return false;
    }
    params.iter().any(|param| param.name == ty.name)
        || ty
            .type_arguments()
            .any(|argument| type_ref_has_ownership_parameter(argument, params))
}

fn type_ref_has_any_ownership_parameter(
    ty: &crate::types::TypeRef,
    function_params: &[ast::TypeParamDecl],
    enclosing_params: &[ast::TypeParamDecl],
) -> bool {
    type_ref_has_ownership_parameter(ty, function_params)
        || type_ref_has_ownership_parameter(ty, enclosing_params)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseMode {
    Read,
    Write,
    Give,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveBorrow {
    root: BorrowPlace,
    mode: UseMode,
    span: Span,
}

#[derive(Debug, Clone)]
struct Flow {
    falls_through: bool,
    backedges: Vec<Scopes>,
    breaks: Vec<Scopes>,
    returns: Vec<Scopes>,
    yields: Vec<Scopes>,
}

#[derive(Debug, Clone)]
struct ExceptionalExit {
    effect: crate::types::ResolvedType,
    site: Span,
    values: HashSet<cleanup::ValueId>,
    scopes: Scopes,
}

#[derive(Debug)]
struct ExceptionScope {
    lexical_depth: usize,
    exits: Vec<ExceptionalExit>,
}

impl Flow {
    fn fallthrough() -> Self {
        Self {
            falls_through: true,
            backedges: Vec::new(),
            breaks: Vec::new(),
            returns: Vec::new(),
            yields: Vec::new(),
        }
    }

    fn stops() -> Self {
        Self {
            falls_through: false,
            backedges: Vec::new(),
            breaks: Vec::new(),
            returns: Vec::new(),
            yields: Vec::new(),
        }
    }

    fn breaks(scopes: &Scopes) -> Self {
        Self {
            falls_through: false,
            backedges: Vec::new(),
            breaks: vec![scopes.clone()],
            returns: Vec::new(),
            yields: Vec::new(),
        }
    }

    fn returns(scopes: &Scopes) -> Self {
        Self {
            falls_through: false,
            backedges: Vec::new(),
            breaks: Vec::new(),
            returns: vec![scopes.clone()],
            yields: Vec::new(),
        }
    }

    fn yields(scopes: &Scopes) -> Self {
        Self {
            falls_through: false,
            backedges: Vec::new(),
            breaks: Vec::new(),
            returns: Vec::new(),
            yields: vec![scopes.clone()],
        }
    }
}

struct Checker<'a> {
    composition: &'a crate::trait_composition::CompositionPlan,
    classes: HashSet<String>,
    receiver_type_params: HashMap<String, Vec<ast::TypeParamDecl>>,
    signatures: HashMap<String, Signature>,
    constructors: HashMap<String, Signature>,
    methods: HashMap<(String, String), Signature>,
    enum_cases: HashMap<(String, String), Signature>,
    properties: HashMap<(String, String), PropertyInfo>,
    static_properties: HashMap<(String, String), bool>,
    inferred_move_returns: HashSet<Span>,
    return_borrows: HashMap<Span, Option<ReturnBorrow>>,
    specialized_return_borrows: &'a crate::semantics::SpecializedReturnBorrows,
    resolved_types: &'a HashMap<Span, crate::types::ResolvedType>,
    interface_conversion_types: &'a HashMap<Span, crate::types::ResolvedType>,
    given_preludes: &'a HashMap<Span, crate::semantics::GivenSemanticInfo>,
    checked_effect_sites: &'a crate::checked_effects::EffectSiteMap,
    catch_error_types: &'a crate::checked_effects::CatchTypeMap,
    catch_coverage: &'a crate::checked_effects::CatchCoverageMap,
    exception_scopes: Vec<ExceptionScope>,
    move_enum_names: MoveEnums,
    move_nominal_names: HashSet<String>,
    receiver_class: Option<String>,
    receiver_writable: bool,
    current_type_params: Vec<ast::TypeParamDecl>,
    current_return_borrow: Option<UseMode>,
    active_assignment_writes: HashSet<BorrowPlace>,
    active_assignment_targets: HashSet<String>,
    active_borrows: Vec<ActiveBorrow>,
    when_result_modes: Vec<UseMode>,
    conditional_borrow_sources: HashMap<Span, ConditionalBorrowSources>,
    flow_facts: &'a FactsByUse,
    binding_resolution: &'a BindingResolution,
    closures: &'a HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    callable_value_calls: &'a HashMap<Span, crate::semantics::CallableValueCallInfo>,
    callback_capture_targets: &'a HashMap<Span, Vec<CallbackCaptureTarget>>,
    callback_value_targets: &'a HashMap<Span, Vec<CallbackCaptureTarget>>,
    assertion_callable_invocations: &'a HashMap<Span, FunctionInvocationMode>,
    list_algorithm_calls: &'a HashMap<Span, crate::semantics::ListAlgorithmCallInfo>,
    call_targets: &'a HashMap<Span, crate::semantics::CallableTarget>,
    property_accessor_calls: &'a HashMap<Span, crate::semantics::PropertyAccessorCalls>,
    property_writes: &'a HashMap<Span, crate::semantics::PropertyWriteSemanticInfo>,
    matches: &'a HashMap<Span, crate::semantics::MatchSemanticInfo>,
    enum_semantics: &'a [crate::semantics::EnumSemanticInfo],
    enum_case_constructions: &'a HashMap<Span, crate::enums::EnumCaseSource>,
    retained: retained::Analysis,
    writeback: writeback::Analysis,
    generic_call_specializations: &'a HashMap<Span, crate::semantics::GenericSpecialization>,
    next_binding_id: usize,
    diagnostics: Vec<Diagnostic>,
    closure_ownership: HashMap<ClosureId, ClosureOwnershipInfo>,
    binding_ownership: HashMap<BindingId, BindingOwnership>,
    writable_borrow_sources: HashSet<Span>,
    closure_values: HashMap<ClosureId, RetainedValueState>,
    analyzed_closures: HashSet<ClosureId>,
    prepared_closure_evaluations: HashSet<ClosureId>,
    cleanup_owner: Option<Span>,
    cleanup_analysis: cleanup::Analysis,
    cleanup_yield_regions: Vec<(Span, usize)>,
}

impl Checker<'_> {
    fn next_binding_id(&mut self) -> OwnershipSlotId {
        let id = OwnershipSlotId(self.next_binding_id);
        self.next_binding_id += 1;
        id
    }

    fn canonical_binding_id(&self, span: Span) -> Option<BindingId> {
        self.binding_resolution
            .declaration_by_span
            .get(&span)
            .copied()
    }

    fn canonical_function_type(
        &self,
        id: Option<BindingId>,
    ) -> Option<crate::types::SemanticFunctionType<ResolvedType>> {
        id.and_then(|id| self.binding_resolution.declarations_by_id.get(&id))
            .and_then(|declaration| declaration.source_type.as_ref())
            .and_then(non_null_function_type)
            .cloned()
    }

    fn parameter_function_value(
        &self,
        id: Option<BindingId>,
        ownership: BindingOwnership,
        take_parameter_insertion: Option<Span>,
    ) -> Option<RetainedValueState> {
        let inherited_parameter = id.is_some_and(|id| {
            self.binding_resolution
                .declarations_by_id
                .get(&id)
                .is_some_and(|declaration| {
                    matches!(
                        declaration.kind,
                        BindingKind::FunctionParameter
                            | BindingKind::MethodParameter
                            | BindingKind::ClosureParameter
                    )
                })
        });
        self.canonical_function_type(id)
            .map(|_| RetainedValueState {
                kind: RetainedValueKind::Closure,
                closure_id: None,
                provenance: if inherited_parameter || ownership != BindingOwnership::Owned {
                    ValueProvenance::BorrowBound(id.map(BorrowRoot::Binding).into_iter().collect())
                } else {
                    ValueProvenance::Owned
                },
                // The caller supplies the actual dependency set. An opaque
                // callback may own a source-bound value whose destructor still
                // needs that set after the callback's last invocation.
                leases: id
                    .filter(|_| inherited_parameter)
                    .map(|id| RetainedLoan {
                        root: BorrowRoot::Binding(id),
                        root_key: format!("retained-parameter:{}", id.0).into(),
                        access: BorrowAccess::Readonly,
                        capture_span: self.binding_resolution.declarations_by_id[&id]
                            .span
                            .unwrap_or_default(),
                        source_depth: 0,
                        inherited: true,
                        lifetime: RetainedLifetime::Cleanup,
                        reborrows: Vec::new(),
                        pending_capture: None,
                    })
                    .into_iter()
                    .collect(),
                nonescaping_parameter: ownership != BindingOwnership::Owned,
                take_parameter_insertion,
                storage: owned_storage::Contents::default(),
                enum_payloads: None,
            })
    }

    fn push_exception_scope(&mut self, scopes: &Scopes) {
        self.exception_scopes.push(ExceptionScope {
            lexical_depth: scopes.lexical_depth(),
            exits: Vec::new(),
        });
    }

    fn pop_exception_scope(&mut self) -> Vec<ExceptionalExit> {
        self.exception_scopes
            .pop()
            .expect("checked-error ownership scope")
            .exits
    }

    fn record_exceptional_exits(&mut self, span: Span, scopes: &Scopes) {
        let failed = self.retained.failed_storage_calls.remove(&span);
        let scopes = failed.as_ref().unwrap_or(scopes);
        self.record_exceptional_effects_at(
            span,
            crate::checked_effects::effects_at(self.checked_effect_sites, span),
            scopes,
        );
    }

    fn record_exceptional_effects_at(
        &mut self,
        span: Span,
        effects: &[ResolvedType],
        scopes: &Scopes,
    ) {
        self.record_exceptional_effects_with_values(span, effects, scopes, None);
    }

    fn record_exceptional_effects_with_values(
        &mut self,
        span: Span,
        effects: &[ResolvedType],
        scopes: &Scopes,
        transferred: Option<&HashSet<cleanup::ValueId>>,
    ) {
        let Some(depth) = self
            .exception_scopes
            .last()
            .map(|scope| scope.lexical_depth)
        else {
            return;
        };
        if depth == 0 && !effects.is_empty() {
            self.summarize_escaping_stored_sources(scopes);
        }
        for effect in effects {
            let values = transferred.cloned().unwrap_or_else(|| {
                self.cleanup_owner
                    .map(|owner| {
                        HashSet::from([self.cleanup_analysis.acquire(
                            owner,
                            cleanup::Source::Expression(span),
                            effect.clone(),
                            HashSet::new(),
                        )])
                    })
                    .unwrap_or_default()
            });
            let mut exit_scopes = scopes.clone();
            let dropped = exit_scopes.1.drain_for_exception(depth);
            self.cleanup_release(span, dropped, cleanup::Cause::CheckedError);
            exit_scopes.truncate_to(depth);
            self.exception_scopes
                .last_mut()
                .expect("exception scope")
                .exits
                .push(ExceptionalExit {
                    effect: effect.clone(),
                    site: span,
                    values,
                    scopes: exit_scopes,
                });
        }
    }

    fn propagate_exceptional_exits(&mut self, exits: impl IntoIterator<Item = ExceptionalExit>) {
        let Some(depth) = self
            .exception_scopes
            .last()
            .map(|scope| scope.lexical_depth)
        else {
            return;
        };
        for mut exit in exits {
            let values = exit.scopes.1.drain_for_exception(depth);
            self.cleanup_release(exit.site, values, cleanup::Cause::CheckedError);
            exit.scopes.truncate_to(depth);
            self.exception_scopes
                .last_mut()
                .expect("parent exception scope")
                .exits
                .push(exit);
        }
    }

    fn apply_finally_to_exceptional_exits(
        &mut self,
        finally: &ast::Block,
        exits: &mut Vec<ExceptionalExit>,
        return_move_type: bool,
    ) -> Flow {
        let diagnostics_before = self.diagnostics.len();
        let mut overrides = Flow::stops();
        exits.retain_mut(|exit| {
            exit.scopes.1.put(
                cleanup::Slot::PendingError(exit.site),
                exit.values.clone(),
                0,
            );
            let mut flow = self.check_block(finally, &mut exit.scopes, return_move_type, true);
            self.cleanup_superseded_control_exit(&mut flow, finally.span);
            if flow.falls_through {
                exit.values = exit.scopes.1.take(&cleanup::Slot::PendingError(exit.site));
            }
            overrides.backedges.append(&mut flow.backedges);
            overrides.breaks.append(&mut flow.breaks);
            overrides.returns.append(&mut flow.returns);
            overrides.yields.append(&mut flow.yields);
            flow.falls_through
        });
        self.deduplicate_diagnostics_from(diagnostics_before);
        overrides
    }

    fn check_function(&mut self, function: &ast::FunctionDecl, receiver_class: Option<&str>) {
        let Some(body) = function.body.as_block() else {
            return;
        };
        self.prepare_retained_liveness(body, function.span);
        let cleanup_context = self.cleanup_begin_callable(function.span);
        let previous_retained_context = self
            .retained
            .enter_function(function, self.binding_resolution);
        let enclosing_type_params = receiver_class
            .and_then(|class| self.receiver_type_params.get(class))
            .cloned()
            .unwrap_or_default();
        let previous_receiver =
            std::mem::replace(&mut self.receiver_class, receiver_class.map(str::to_owned));
        let previous_receiver_writable = std::mem::replace(
            &mut self.receiver_writable,
            function.has_writable_receiver(),
        );
        let mut current_type_params = function.type_params.clone();
        current_type_params.extend(enclosing_type_params.clone());
        let previous_type_params =
            std::mem::replace(&mut self.current_type_params, current_type_params);
        let previous_return_borrow = self.current_return_borrow;
        self.current_return_borrow = function
            .return_type
            .as_ref()
            .is_some_and(|ty| {
                type_ref_is_move_type_with_nominals(
                    ty,
                    &self.classes,
                    &self.move_nominal_names,
                    self.receiver_class.as_deref(),
                ) || type_ref_has_any_ownership_parameter(
                    ty,
                    &function.type_params,
                    &enclosing_type_params,
                )
            })
            .then(|| {
                self.return_borrows
                    .get(&function.span)
                    .copied()
                    .unwrap_or_else(|| {
                        function_return_borrow_with_calls(
                            function,
                            return_borrow_parameters(function.params.iter().map(|param| {
                                let symbolic = type_ref_has_any_ownership_parameter(
                                    &param.ty,
                                    &function.type_params,
                                    &enclosing_type_params,
                                );
                                (
                                    !param.take
                                        && (type_ref_is_move_type_with_nominals(
                                            &param.ty,
                                            &self.classes,
                                            &self.move_nominal_names,
                                            self.receiver_class.as_deref(),
                                        ) || symbolic),
                                    symbolic,
                                )
                            })),
                            &mut |_| ReturnBorrowLookup::Unresolved,
                        )
                    })
            })
            .flatten()
            .filter(|borrow| borrow.kind == ReturnBorrowKind::Value)
            .map(|borrow| {
                if borrow.writable {
                    UseMode::Write
                } else {
                    UseMode::Read
                }
            });
        let mut scopes = Scopes::new();
        if function.consuming_this {
            let canonical_id = self.binding_resolution.receiver_for_callable(function.span);
            let retained_value = self.iterator_value_from_expr(
                &Expr::This {
                    span: function.span,
                },
                &scopes,
            );
            scopes.declare(
                "this".to_string(),
                Binding {
                    id: self.next_binding_id(),
                    canonical_id,
                    class: receiver_class.map(str::to_owned),
                    collection: None,
                    mixed: false,
                    borrowed_place: false,
                    borrow_root: Some(BorrowSourceRoot::Place("$this".into())),
                    borrow_parents: Vec::new(),
                    writeback_homes: Vec::new(),
                    writable: false,
                    state: State::Owned,
                    function_type: None,
                    retained_value,
                    scope_depth: scopes.lexical_depth(),
                },
            );
            if let Some(binding) = canonical_id {
                self.cleanup_owned_parameter(binding, &mut scopes);
            }
        }
        for param in &function.params {
            let class =
                type_ref_class_name(&param.ty, &self.classes, self.receiver_class.as_deref());
            let mixed = param.ty.name == "mixed";
            let canonical_id = self.canonical_binding_id(param.span);
            let function_type = self.canonical_function_type(canonical_id);
            let copy_parameter = canonical_id
                .and_then(|id| self.binding_resolution.declarations_by_id.get(&id))
                .and_then(|declaration| declaration.source_type.as_ref())
                .is_some_and(|ty| {
                    !resolved_type_requires_conservative_move(ty)
                        && !resolved_type_is_move_type(ty, &self.move_enum_names)
                });
            let promotion_transfers_parameter =
                param.take && param.constructor_role.is_promoted() && !copy_parameter;
            let ownership = if param.take {
                BindingOwnership::Owned
            } else if param.writable {
                BindingOwnership::WritableBorrow
            } else {
                BindingOwnership::ReadonlyBorrow
            };
            let retained_value = self
                .parameter_function_value(
                    canonical_id,
                    ownership,
                    (!param.take && !param.writable).then_some(param.ownership_modifier_insert),
                )
                .or_else(|| self.parameter_retained_value(canonical_id));
            if function.name == "__construct"
                && param.constructor_role.is_promoted()
                && param.borrow_span.is_none()
            {
                if let Some(value) = retained_value.as_ref() {
                    self.validate_constructed_property_storage(
                        value,
                        param.span,
                        "an owned promoted property",
                        true,
                    );
                }
            }
            scopes.declare(
                param.name.clone(),
                Binding {
                    id: self.next_binding_id(),
                    canonical_id,
                    class,
                    collection: type_ref_collection_info(
                        &param.ty,
                        &self.classes,
                        &self.move_nominal_names,
                        self.receiver_class.as_deref(),
                        &self.current_type_params,
                        &[],
                    ),
                    mixed,
                    borrowed_place: !param.take,
                    borrow_root: None,
                    borrow_parents: Vec::new(),
                    writeback_homes: Vec::new(),
                    writable: param.writable,
                    state: if promotion_transfers_parameter {
                        State::Given { at: param.span }
                    } else if param.take {
                        State::Owned
                    } else {
                        State::Borrowed
                    },
                    function_type,
                    retained_value,
                    scope_depth: scopes.lexical_depth(),
                },
            );
            if param.take && !promotion_transfers_parameter {
                if let Some(binding) = canonical_id {
                    self.cleanup_owned_parameter(binding, &mut scopes);
                }
            }
        }
        self.prepare_conditional_borrow_sources(function, &scopes);
        let return_move_type = function.return_type.as_ref().is_some_and(|ty| {
            (type_ref_is_move_type_with_nominals(
                ty,
                &self.classes,
                &self.move_nominal_names,
                self.receiver_class.as_deref(),
            ) || type_ref_has_any_ownership_parameter(ty, &self.current_type_params, &[]))
                && self.current_return_borrow.is_none()
        }) || (function.return_type.is_none()
            && self.inferred_move_returns.contains(&function.span));
        let mut flow = self.check_block(body, &mut scopes, return_move_type, false);
        self.summarize_stored_call_sources(&scopes, &flow);
        self.cleanup_end_callable(cleanup_context, &mut scopes, &mut flow, body.span);
        self.current_return_borrow = previous_return_borrow;
        self.current_type_params = previous_type_params;
        self.receiver_writable = previous_receiver_writable;
        self.receiver_class = previous_receiver;
        self.retained.leave_callable(previous_retained_context);
    }

    fn prepare_conditional_borrow_sources(
        &mut self,
        function: &ast::FunctionDecl,
        scopes: &Scopes,
    ) {
        let Some(body) = function.body.as_block() else {
            return;
        };
        let mut has_conditional = false;
        crate::ast::visit::block(body, &mut |expr| {
            has_conditional |= matches!(expr, Expr::When(_) | Expr::Match { .. });
        });
        if !has_conditional {
            return;
        }
        let graph = crate::control_flow::build_function_cfg_with_checked_effects(
            body,
            function.span,
            self.given_preludes,
            self.checked_effect_sites,
            self.catch_error_types,
            self.catch_coverage,
            &HashSet::new(),
        );
        let parameters = return_borrow_parameters(function.params.iter().map(|parameter| {
            let symbolic =
                type_ref_has_any_ownership_parameter(&parameter.ty, &self.current_type_params, &[]);
            (
                !parameter.take
                    && (symbolic
                        || type_ref_is_move_type_with_nominals(
                            &parameter.ty,
                            &self.classes,
                            &self.move_nominal_names,
                            self.receiver_class.as_deref(),
                        )),
                symbolic,
            )
        }));
        let inferred = infer_return_borrows_with_bindings(
            function,
            parameters,
            &mut |expr| {
                if self.creates_owned_interface_temporary(expr) {
                    return ReturnBorrowLookup::OwnedValue;
                }
                let Some((signature, _, args)) = self.retained_call(expr, scopes) else {
                    return ReturnBorrowLookup::Unresolved;
                };
                match signature.return_borrow {
                    Some(
                        borrow @ ReturnBorrow {
                            source: BorrowSource::Parameter(index),
                            ..
                        },
                    ) => signature
                        .bind_arguments(args)
                        .param_to_arg
                        .get(index)
                        .copied()
                        .flatten()
                        .map_or(ReturnBorrowLookup::Resolved(None), |argument_index| {
                            ReturnBorrowLookup::ResolvedArgument {
                                borrow,
                                argument_index,
                            }
                        }),
                    borrow => ReturnBorrowLookup::Resolved(borrow),
                }
            },
            self.binding_resolution,
            &graph,
        );
        for span in graph.value_results.keys() {
            self.conditional_borrow_sources.remove(span);
        }
        for (span, origins) in inferred.conditionals {
            let bindings = origins
                .sources
                .iter()
                .map(|source| match source {
                    BorrowOrigin::Receiver => Some(None),
                    BorrowOrigin::Parameter(index) => function
                        .params
                        .get(*index)
                        .and_then(|parameter| self.canonical_binding_id(parameter.span))
                        .map(Some),
                    BorrowOrigin::Local(id) => Some(Some(*id)),
                })
                .collect::<Option<Vec<_>>>();
            if let Some(bindings) = bindings {
                self.conditional_borrow_sources
                    .insert(span, ConditionalBorrowSources { origins, bindings });
            }
        }
    }

    fn check_block(
        &mut self,
        block: &ast::Block,
        scopes: &mut Scopes,
        return_move_type: bool,
        nested: bool,
    ) -> Flow {
        if nested {
            scopes.push();
        }
        let mut flow = Flow::fallthrough();
        for (index, statement) in block.statements.iter().enumerate() {
            if !flow.falls_through {
                break;
            }
            scopes.release_unused_borrows(&block.statements[index..], nested);
            scopes.release_unused_retained_loans(
                &block.statements[index..],
                nested,
                self.retained_live_bindings(cleanup::statement_span(statement)),
            );
            let borrow_depth = self.active_borrows.len();
            self.active_borrows.extend(scopes.1.borrows());
            let mut statement_flow = self.check_statement(statement, scopes, return_move_type);
            self.active_borrows.truncate(borrow_depth);
            let site = cleanup::statement_span(statement);
            if statement_flow.falls_through {
                self.cleanup_release_temporaries(scopes, site);
            }
            for exit in statement_flow
                .backedges
                .iter_mut()
                .chain(&mut statement_flow.breaks)
                .chain(&mut statement_flow.returns)
                .chain(&mut statement_flow.yields)
            {
                self.cleanup_release_temporaries(exit, site);
            }
            flow.falls_through = statement_flow.falls_through;
            flow.backedges.extend(statement_flow.backedges);
            flow.breaks.extend(statement_flow.breaks);
            flow.returns.extend(statement_flow.returns);
            flow.yields.extend(statement_flow.yields);
        }
        if nested {
            self.cleanup_pop_flow_scope(scopes, &mut flow, block.span);
        }
        flow
    }

    fn check_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Flow {
        match statement {
            Stmt::Block(block) => self.check_block(block, scopes, return_move_type, true),
            Stmt::VarDecl(decl) => {
                let declaration_name = decl
                    .bindings
                    .first()
                    .map(|binding| binding.name.as_str())
                    .unwrap_or("local");
                let declared_class = decl.ty.as_ref().and_then(|ty| {
                    type_ref_class_name(ty, &self.classes, self.receiver_class.as_deref())
                });
                let class = declared_class.or_else(|| self.expr_class(&decl.initializer, scopes));
                let function_type = decl
                    .bindings
                    .first()
                    .and_then(|binding| self.canonical_binding_id(binding.span))
                    .and_then(|binding| self.canonical_function_type(Some(binding)))
                    .or_else(|| {
                        decl.ty.is_none().then(|| {
                            self.resolved_type(&decl.initializer)
                                .and_then(non_null_function_type)
                                .cloned()
                        })?
                    });
                let borrowed_carrier_alias = self.borrowed_carrier_alias(&decl.initializer, scopes);
                let borrowed_initializer =
                    self.expr_returns_borrow(&decl.initializer, scopes) || borrowed_carrier_alias;
                let borrowed_carrier = borrowed_initializer;
                let borrowed_mixed_index = borrowed_initializer
                    && self.expr_is_mixed_collection_index(&decl.initializer, scopes);
                let initializer_source = self.borrow_source_root(&decl.initializer, scopes);
                // A readonly alias of an owner-free returned value retains its
                // source fact even though no active loan remains. A retained
                // callback owns its environment separately, and an ordinary
                // nullable local initialized from null still has its own place.
                let immutable_null_alias = !decl.writable
                    && function_type.is_none()
                    && initializer_source == Some(BorrowSourceRoot::ImmutableNull)
                    && (self.expr_has_return_borrow_contract(&decl.initializer, scopes)
                        || variable_name(&decl.initializer).is_some());
                let borrow_root = (borrowed_carrier || immutable_null_alias)
                    .then_some(initializer_source)
                    .flatten();
                let initializer_moves = self.expr_is_move_value(&decl.initializer, scopes);
                let mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                    || (decl.ty.is_none()
                        && class.is_none()
                        && function_type.is_none()
                        && initializer_moves);
                let declared_move_type = decl.ty.as_ref().is_some_and(|ty| {
                    type_ref_is_move_type_with_nominals(
                        ty,
                        &self.classes,
                        &self.move_nominal_names,
                        self.receiver_class.as_deref(),
                    )
                });
                let borrowed_owning_value = borrowed_carrier
                    && !borrowed_mixed_index
                    && (initializer_moves || class.is_some() || mixed || declared_move_type);
                let invalid_borrow = borrowed_owning_value && borrow_root.is_none();
                let explicit_owning_borrow =
                    borrowed_carrier && declared_move_type && function_type.is_none();
                if explicit_owning_borrow {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0478",
                            format!(
                                "borrowed result cannot initialize owning `${}`",
                                declaration_name
                            ),
                            decl.initializer.span(),
                        )
                        .with_help(
                            "use an inferred `let` binding for a borrow, or initialize this declaration with an independently owned value",
                        ),
                    );
                } else if invalid_borrow {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0478",
                            format!(
                                "borrowed result from a temporary cannot initialize `${}`",
                                declaration_name
                            ),
                            decl.initializer.span(),
                        )
                        .with_help(
                            "keep the borrowed owner in a local for at least as long as this binding",
                        ),
                    );
                }
                let writable_borrow = borrowed_owning_value
                    && decl.writable
                    && self.borrowed_value_is_writable(&decl.initializer, scopes);
                if borrowed_owning_value && decl.writable && !writable_borrow {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0478",
                            format!("readonly borrowed result cannot initialize writable `${declaration_name}`"),
                            decl.span,
                        )
                        .with_help(
                            "remove `writable`, or initialize this binding from a proved writable borrow",
                        ),
                    );
                }
                let retained_value = self.evaluate_retained_value(
                    &decl.initializer,
                    scopes,
                    if writable_borrow {
                        UseMode::Write
                    } else if borrowed_owning_value {
                        UseMode::Read
                    } else if initializer_moves || class.is_some() || mixed || declared_move_type {
                        UseMode::Give
                    } else {
                        UseMode::Read
                    },
                );
                let function_destination_valid = retained_value.as_ref().is_none_or(|value| {
                    if decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed") {
                        self.validate_owned_function_storage(
                            value,
                            decl.initializer.span(),
                            "`mixed`",
                        )
                    } else {
                        self.validate_local_function_destination(
                            value,
                            scopes.lexical_depth(),
                            decl.initializer.span(),
                        )
                    }
                });
                if function_type.is_some() && decl.bindings.len() > 1 {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0655",
                            "one function value cannot initialize multiple bindings",
                            decl.span,
                        )
                        .with_title("Function Value Cannot Be Copied")
                        .with_help("initialize each binding with a distinct closure value"),
                    );
                }
                {
                    let collection = decl
                        .ty
                        .as_ref()
                        .and_then(|ty| {
                            type_ref_collection_info(
                                ty,
                                &self.classes,
                                &self.move_nominal_names,
                                self.receiver_class.as_deref(),
                                &self.current_type_params,
                                &[],
                            )
                        })
                        .or_else(|| self.expr_collection_info(&decl.initializer, scopes));
                    for (index, declaration) in decl.bindings.iter().enumerate() {
                        let canonical_id = self.canonical_binding_id(declaration.span);
                        let writeback_homes = if borrowed_owning_value {
                            self.writeback_homes(&decl.initializer, scopes)
                        } else {
                            Vec::new()
                        };
                        if let Some(binding) = canonical_id {
                            self.binding_ownership.insert(
                                binding,
                                if borrowed_owning_value {
                                    if writable_borrow {
                                        BindingOwnership::WritableBorrow
                                    } else {
                                        BindingOwnership::ReadonlyBorrow
                                    }
                                } else {
                                    BindingOwnership::Owned
                                },
                            );
                        }
                        scopes.declare(
                            declaration.name.clone(),
                            Binding {
                                id: self.next_binding_id(),
                                canonical_id,
                                class: class.clone(),
                                collection: collection.clone(),
                                mixed,
                                borrowed_place: borrowed_owning_value,
                                // Fresh callback environments remain owned even
                                // with retained loans; an existing carrier lent
                                // by a getter or alias remains borrowed.
                                borrow_root: borrow_root.clone(),
                                borrow_parents: if borrowed_owning_value {
                                    self.borrow_input_bindings(&decl.initializer, scopes)
                                } else {
                                    Vec::new()
                                },
                                writeback_homes,
                                writable: decl.writable,
                                state: if function_type.is_some() && index > 0 {
                                    State::Given { at: decl.span }
                                } else if borrowed_owning_value {
                                    State::Borrowed
                                } else {
                                    State::Owned
                                },
                                function_type: self
                                    .canonical_function_type(canonical_id)
                                    .or_else(|| function_type.clone()),
                                retained_value: ((index == 0 || !initializer_moves)
                                    && function_destination_valid)
                                    .then(|| retained_value.clone())
                                    .flatten(),
                                scope_depth: scopes.lexical_depth(),
                            },
                        );
                        if index == 0 && !borrowed_owning_value {
                            if let Some(binding) = canonical_id {
                                self.cleanup_bind_result(
                                    binding,
                                    &decl.initializer,
                                    scopes,
                                    decl.span,
                                );
                            }
                        }
                    }
                }
                Flow::fallthrough()
            }
            Stmt::Assignment(assignment) => {
                self.check_pending_storage_access(&assignment.target, UseMode::Write, scopes);
                if assignment.op == AssignOp::Assign {
                    self.record_writeback_assignment(
                        &assignment.target,
                        &assignment.value,
                        scopes,
                        assignment.span,
                    );
                }
                if assignment.op == AssignOp::Assign
                    && self.use_property_setter(&assignment.target, &assignment.value, scopes)
                {
                    return Flow::fallthrough();
                }
                if assignment.op != AssignOp::Assign {
                    if self.use_property_update(&assignment.target, Some(&assignment.value), scopes)
                    {
                        return Flow::fallthrough();
                    }
                    self.use_assignment_operands(&assignment.target, &assignment.value, scopes);
                    return Flow::fallthrough();
                }
                if let Expr::Variable { name, span } = &assignment.target {
                    let through = self.borrow_input_bindings(&assignment.target, scopes);
                    for root in self.borrow_root_keys(&assignment.target, scopes) {
                        self.check_borrow_root_access(
                            &root,
                            UseMode::Write,
                            assignment.target.span(),
                            &through,
                            scopes,
                        );
                    }
                    let value_is_function = self
                        .resolved_type(&assignment.value)
                        .and_then(non_null_function_type)
                        .is_some()
                        || self
                            .retained_value_from_expr(&assignment.value, scopes)
                            .is_some_and(|value| value.kind == RetainedValueKind::Closure);
                    if self.expr_returns_borrow(&assignment.value, scopes)
                        && scopes.get(name).is_some()
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0478",
                                format!("borrowed result cannot replace owning `${name}`"),
                                assignment.value.span(),
                            )
                            .with_help(
                                "keep using the result in the current expression, or assign an independently owned value",
                            ),
                        );
                        return Flow::fallthrough();
                    }
                    let value_class = self.expr_class(&assignment.value, scopes);
                    let value_moves = self.expr_is_move_value(&assignment.value, scopes);
                    let target = scopes.get(name).cloned();
                    let target_is_function = target
                        .as_ref()
                        .is_some_and(|binding| binding.function_type.is_some());
                    let function_assignment = target_is_function || value_is_function;
                    let class_assignment = value_class.is_some()
                        && target
                            .as_ref()
                            .is_some_and(|binding| binding.mixed || binding.class == value_class);
                    let mixed_assignment = target.as_ref().is_some_and(|binding| binding.mixed);
                    let move_assignment = target.is_some() && value_moves;
                    if mixed_assignment && value_is_function {
                        let pending_value = self.prepare_retained_value(&assignment.value, scopes);
                        let mut valid = pending_value.as_ref().is_none_or(|value| {
                            self.validate_owned_function_storage(
                                value,
                                assignment.value.span(),
                                "`mixed`",
                            )
                        });
                        self.use_expr(
                            &assignment.value,
                            scopes,
                            if valid { UseMode::Give } else { UseMode::Read },
                        );
                        let evaluated = self.retained_value_from_expr(&assignment.value, scopes);
                        if pending_value.is_none() {
                            valid &= evaluated.as_ref().is_none_or(|value| {
                                self.validate_owned_function_storage(
                                    value,
                                    assignment.value.span(),
                                    "`mixed`",
                                )
                            });
                        }
                        if valid {
                            if let Some(binding) =
                                target.as_ref().and_then(|binding| binding.canonical_id)
                            {
                                self.cleanup_bind_result(
                                    binding,
                                    &assignment.value,
                                    scopes,
                                    assignment.span,
                                );
                            }
                            if let Some(binding) = scopes.get_mut(name) {
                                binding.state = State::Owned;
                                binding.retained_value = evaluated;
                            }
                        }
                        return Flow::fallthrough();
                    }
                    if function_assignment {
                        if variable_name(&assignment.value).is_some_and(|source| source == name) {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0471",
                                    format!("`${name}` cannot be given to itself"),
                                    assignment.span,
                                )
                                .with_help("move the function value to a different destination"),
                            );
                            return Flow::fallthrough();
                        }
                        let writable = target.as_ref().is_some_and(|binding| binding.writable);
                        if !writable {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0656",
                                    format!("readonly function-value binding `${name}` cannot be replaced"),
                                    *span,
                                )
                                .with_title("Readonly Function Value Cannot Be Reassigned")
                                .with_help("declare the binding `writable` when it must be replaced"),
                            );
                            self.use_expr(&assignment.value, scopes, UseMode::Read);
                            return Flow::fallthrough();
                        }
                        let pending_value = self.evaluate_retained_value(
                            &assignment.value,
                            scopes,
                            if matches!(&assignment.value, Expr::Null { .. }) {
                                UseMode::Read
                            } else {
                                UseMode::Give
                            },
                        );
                        if !matches!(&assignment.value, Expr::Null { .. })
                            && pending_value.is_none()
                        {
                            return Flow::fallthrough();
                        }
                        let target_depth = target
                            .as_ref()
                            .map_or_else(|| scopes.lexical_depth(), |binding| binding.scope_depth);
                        if let Some(value) = pending_value.as_ref() {
                            if !self.validate_local_function_destination(
                                value,
                                target_depth,
                                assignment.value.span(),
                            ) {
                                return Flow::fallthrough();
                            }
                        }
                        if let Some(target) = &target {
                            if target.borrowed_place {
                                self.cleanup_store_place(
                                    &assignment.target,
                                    &assignment.value,
                                    scopes,
                                    assignment.span,
                                    true,
                                );
                            } else if let Some(binding) = target.canonical_id {
                                self.cleanup_bind_result(
                                    binding,
                                    &assignment.value,
                                    scopes,
                                    assignment.span,
                                );
                            }
                        }
                        if let Some(binding) = scopes.get_mut(name) {
                            binding.state = if binding.borrowed_place {
                                State::Borrowed
                            } else {
                                State::Owned
                            };
                            binding.retained_value = pending_value;
                        }
                        return Flow::fallthrough();
                    }
                    if class_assignment || mixed_assignment || move_assignment {
                        if variable_name(&assignment.value).is_some_and(|source| source == name) {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0471",
                                    format!("`${name}` cannot be given to itself"),
                                    assignment.span,
                                )
                                .with_help("give the value to a different owning destination"),
                            );
                            return Flow::fallthrough();
                        }
                        let was_owned = scopes
                            .get(name)
                            .is_some_and(|binding| binding.state == State::Owned);
                        let writable = scopes.get(name).is_some_and(|binding| binding.writable);
                        if !was_owned && !writable {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0473",
                                    format!("readonly `${name}` cannot be assigned a new owner"),
                                    *span,
                                )
                                .with_help(
                                    "declare the binding `writable` if it must be reinitialized after giving its value away",
                                ),
                            );
                        }
                        let pending_value = self.evaluate_retained_value(
                            &assignment.value,
                            scopes,
                            if value_moves || class_assignment {
                                UseMode::Give
                            } else {
                                UseMode::Read
                            },
                        );
                        if let Some(value) = &pending_value {
                            if mixed_assignment {
                                self.validate_owned_function_storage(
                                    value,
                                    assignment.value.span(),
                                    "`mixed`",
                                );
                            } else {
                                let target_depth = target
                                    .as_ref()
                                    .map_or(scopes.lexical_depth(), |binding| binding.scope_depth);
                                self.validate_local_function_destination(
                                    value,
                                    target_depth,
                                    assignment.value.span(),
                                );
                            }
                        }
                        if let Some(target) = &target {
                            if target.borrowed_place {
                                self.cleanup_store_place(
                                    &assignment.target,
                                    &assignment.value,
                                    scopes,
                                    assignment.span,
                                    true,
                                );
                            } else if let Some(binding) = target.canonical_id {
                                self.cleanup_bind_result(
                                    binding,
                                    &assignment.value,
                                    scopes,
                                    assignment.span,
                                );
                            }
                        }
                        if let Some(binding) = scopes.get_mut(name) {
                            binding.retained_value = pending_value;
                            binding.state = if binding.borrowed_place {
                                State::Borrowed
                            } else {
                                State::Owned
                            };
                            if binding.mixed {
                                binding.class = value_class;
                            }
                        }
                    } else {
                        let retained_value =
                            self.evaluate_retained_value(&assignment.value, scopes, UseMode::Read);
                        let target_depth = target
                            .as_ref()
                            .map_or(scopes.lexical_depth(), |binding| binding.scope_depth);
                        if retained_value.as_ref().is_none_or(|value| {
                            self.validate_local_function_destination(
                                value,
                                target_depth,
                                assignment.value.span(),
                            )
                        }) {
                            if let Some(binding) = scopes.get_mut(name) {
                                binding.retained_value = retained_value;
                            }
                        }
                    }
                } else {
                    let static_function_storage =
                        matches!(ungroup_expr(&assignment.target), Expr::StaticMember { .. })
                            && self
                                .resolved_type(&assignment.target)
                                .and_then(non_null_function_type)
                                .is_some();
                    if static_function_storage {
                        if let Some(value) = self.prepare_retained_value(&assignment.value, scopes)
                        {
                            self.reject_deferred_function_storage(
                                &value,
                                assignment.value.span(),
                                "static property storage",
                                "Static Function-Value Storage Is Not Yet Available",
                            );
                        }
                        self.use_assignment_operands_with_mode(
                            &assignment.target,
                            &assignment.value,
                            scopes,
                            UseMode::Read,
                        );
                        return Flow::fallthrough();
                    }
                    let indexed_slot = match ungroup_expr(&assignment.target) {
                        Expr::Index { collection, .. } => {
                            self.expr_collection_info(collection, scopes)
                        }
                        _ => None,
                    };
                    if let Some(slot) = indexed_slot {
                        let borrowed_value = self.expr_returns_borrow(&assignment.value, scopes);
                        let retained_value = self.prepare_retained_value(&assignment.value, scopes);
                        let Expr::Index { collection, .. } = ungroup_expr(&assignment.target)
                        else {
                            unreachable!()
                        };
                        let valid_function_storage = retained_value.as_ref().is_none_or(|value| {
                            self.validate_collection_destination(
                                collection,
                                value,
                                assignment.value.span(),
                                scopes,
                            )
                        });
                        if borrowed_value && slot.value_move && !slot.value_mixed {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0478",
                                    "borrowed result cannot be stored in an owning collection slot",
                                    assignment.value.span(),
                                )
                                .with_help(
                                    "store an independently owned value in the collection instead",
                                ),
                            );
                        }
                        self.use_assignment_operands_with_mode(
                            &assignment.target,
                            &assignment.value,
                            scopes,
                            if borrowed_value || !valid_function_storage {
                                UseMode::Read
                            } else if slot.value_move || slot.value_mixed {
                                UseMode::Give
                            } else {
                                UseMode::Read
                            },
                        );
                        if !borrowed_value
                            && valid_function_storage
                            && (slot.value_move || slot.value_mixed)
                        {
                            if let Some(value) =
                                self.retained_value_from_expr(&assignment.value, scopes)
                            {
                                self.store_collection_sources(
                                    collection,
                                    &value,
                                    assignment.target.span(),
                                    scopes,
                                );
                            }
                            self.cleanup_store_place(
                                &assignment.target,
                                &assignment.value,
                                scopes,
                                assignment.span,
                                true,
                            );
                        }
                        return Flow::fallthrough();
                    }
                    let property = self.assignment_property_info(&assignment.target, scopes);
                    let owning_property = property
                        .as_ref()
                        .is_some_and(|property| property.move_type || property.mixed);
                    let retained_value = self.prepare_retained_value(&assignment.value, scopes);
                    let mut valid_function_storage = true;
                    if let Some(value) = retained_value.as_ref() {
                        let stores_on_receiver = matches!(
                            ungroup_expr(&assignment.target),
                            Expr::PropertyAccess { object, .. }
                                if matches!(ungroup_expr(object), Expr::This { .. })
                        );
                        if stores_on_receiver
                            && matches!(
                                &value.provenance,
                                ValueProvenance::BorrowBound(roots)
                                    if roots.contains(&BorrowRoot::Receiver)
                            )
                        {
                            valid_function_storage = false;
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0658",
                                    "closure cannot borrow `$this` through a property stored on the same receiver",
                                    assignment.value.span(),
                                )
                                .with_title("Closure Cannot Borrow Its Stored Receiver")
                                .with_help("store a fully owned closure or keep the receiver-borrowing closure local"),
                            );
                        } else {
                            valid_function_storage = self.validate_constructed_property_storage(
                                value,
                                assignment.value.span(),
                                "an instance property",
                                stores_on_receiver,
                            );
                        }
                    }
                    let borrowed_value = self.expr_returns_borrow(&assignment.value, scopes);
                    if borrowed_value && owning_property {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0478",
                                "borrowed result cannot be stored in an owning property",
                                assignment.value.span(),
                            )
                            .with_title("Owning Property Needs An Owned Value")
                            .with_explanation(
                                "The property owns its stored value, but this expression provides only a borrow.",
                            )
                            .with_help(
                                "move an owned local or `take` parameter, construct a fresh value, or return an owned value from a helper",
                            ),
                        );
                    }
                    self.use_assignment_operands_with_mode(
                        &assignment.target,
                        &assignment.value,
                        scopes,
                        if owning_property && !borrowed_value && valid_function_storage {
                            UseMode::Give
                        } else {
                            UseMode::Read
                        },
                    );
                    if owning_property && !borrowed_value && valid_function_storage {
                        if let Some(write) = self
                            .property_writes
                            .get(&assignment.target.span())
                            .or_else(|| self.property_writes.get(&assignment.span))
                        {
                            self.cleanup_store_place(
                                &assignment.target,
                                &assignment.value,
                                scopes,
                                assignment.span,
                                write.kind != crate::semantics::PropertyWriteKind::Initialize,
                            );
                        }
                    }
                }
                Flow::fallthrough()
            }
            Stmt::Echo { expr, span } => {
                self.use_expr(expr, scopes, UseMode::Read);
                self.record_exceptional_exits(*span, scopes);
                Flow::fallthrough()
            }
            Stmt::Expr { expr, .. } => {
                self.use_expr(expr, scopes, UseMode::Read);
                if is_panic_expr(expr) {
                    Flow::stops()
                } else {
                    Flow::fallthrough()
                }
            }
            Stmt::Return { expr, span } => {
                if let Some(expr) = expr {
                    if let Some(mode) = self.when_result_modes.last().copied() {
                        if mode == UseMode::Give && self.expr_returns_borrow(expr, scopes) {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0478",
                                    "borrowed result cannot satisfy an owning `when` result",
                                    expr.span(),
                                )
                                .with_help("yield an independently owned value from this branch"),
                            );
                            self.use_expr(expr, scopes, UseMode::Read);
                        } else {
                            let value = self.evaluate_retained_value(expr, scopes, mode);
                            let previous = self
                                .retained
                                .yielded_values
                                .last_mut()
                                .expect("when result");
                            *previous = join_retained_value(previous.as_ref(), value.as_ref());
                        }
                        self.cleanup_stage_yield(expr, scopes);
                        return Flow::yields(scopes);
                    }
                    let prepared = self.prepare_retained_value(expr, scopes);
                    if let Some(value) = &prepared {
                        if !self.validate_returned_function_value(value, expr.span()) {
                            self.use_expr(expr, scopes, UseMode::Read);
                            return Flow::stops();
                        }
                    }
                    if self.reject_returned_temporary_borrow(expr, scopes) {
                        self.use_expr(expr, scopes, UseMode::Read);
                        return Flow::stops();
                    }
                    if return_move_type
                        && self.current_return_borrow.is_none()
                        && self.expr_returns_borrow(expr, scopes)
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0478",
                                "borrowed result cannot satisfy an owning return",
                                expr.span(),
                            )
                            .with_help(
                                "return an independently owned value or return the borrowed source directly",
                            ),
                        );
                        self.use_expr(expr, scopes, UseMode::Read);
                        return Flow::stops();
                    }
                    self.use_expr(
                        expr,
                        scopes,
                        if return_move_type {
                            UseMode::Give
                        } else {
                            self.current_return_borrow.unwrap_or(UseMode::Read)
                        },
                    );
                    self.record_writeback_return(expr, scopes);
                    if let Some(value) = self.retained_value_from_expr(expr, scopes) {
                        // Nested calls supply their selectors during evaluation.
                        self.record_returned_storage_selectors(&value);
                        if prepared.is_none() {
                            self.validate_returned_function_value(&value, expr.span());
                        }
                    }
                }
                self.cleanup_stage_return(expr.as_ref(), scopes, *span);
                Flow::returns(scopes)
            }
            Stmt::If(statement) => {
                if let Some(given) = &statement.given {
                    scopes.push();
                    self.check_given_setup(given, scopes, return_move_type);
                    let mut attached = statement.clone();
                    attached.given = None;
                    let mut flow =
                        self.check_statement(&Stmt::If(attached), scopes, return_move_type);
                    self.cleanup_pop_flow_scope(scopes, &mut flow, statement.span);
                    return flow;
                }
                if let Some(finally) = &statement.finally {
                    let mut attached = statement.clone();
                    attached.finally = None;
                    let flow = self.check_statement(&Stmt::If(attached), scopes, return_move_type);
                    return self.apply_finally_to_flow(
                        &finally.block,
                        scopes,
                        flow,
                        return_move_type,
                    );
                }
                self.use_expr(&statement.condition, scopes, UseMode::Read);
                self.cleanup_release_temporaries(scopes, statement.condition.span());
                if let Some(condition) = constant_bool(&statement.condition) {
                    if condition {
                        return self.check_block(
                            &statement.then_block,
                            scopes,
                            return_move_type,
                            true,
                        );
                    }
                    return if let Some(branch) = &statement.else_branch {
                        match branch {
                            ast::ElseBranch::If(nested) => self.check_statement(
                                &Stmt::If((**nested).clone()),
                                scopes,
                                return_move_type,
                            ),
                            ast::ElseBranch::Block(block) => {
                                self.check_block(block, scopes, return_move_type, true)
                            }
                        }
                    } else {
                        Flow::fallthrough()
                    };
                }
                let before = scopes.clone();
                let mut then_scopes = before.clone();
                let mut then_flow = self.check_block(
                    &statement.then_block,
                    &mut then_scopes,
                    return_move_type,
                    true,
                );
                let mut else_scopes = before.clone();
                let mut else_flow = if let Some(branch) = &statement.else_branch {
                    match branch {
                        ast::ElseBranch::If(nested) => self.check_statement(
                            &Stmt::If((**nested).clone()),
                            &mut else_scopes,
                            return_move_type,
                        ),
                        ast::ElseBranch::Block(block) => {
                            self.check_block(block, &mut else_scopes, return_move_type, true)
                        }
                    }
                } else {
                    Flow::fallthrough()
                };
                match (then_flow.falls_through, else_flow.falls_through) {
                    (true, true) => scopes.merge_from(&then_scopes, &else_scopes),
                    (true, false) => *scopes = then_scopes,
                    (false, true) => *scopes = else_scopes,
                    (false, false) => *scopes = before,
                }
                then_flow.backedges.append(&mut else_flow.backedges);
                then_flow.breaks.append(&mut else_flow.breaks);
                then_flow.returns.append(&mut else_flow.returns);
                then_flow.yields.append(&mut else_flow.yields);
                Flow {
                    falls_through: then_flow.falls_through || else_flow.falls_through,
                    backedges: then_flow.backedges,
                    breaks: then_flow.breaks,
                    returns: then_flow.returns,
                    yields: then_flow.yields,
                }
            }
            Stmt::While(statement) => {
                if let Some(given) = &statement.given {
                    scopes.push();
                    let predicates = self.check_given_setup(given, scopes, return_move_type);
                    let mut attached = statement.clone();
                    attached.given = None;
                    let mut flow = self.check_while_with_finally(
                        &attached,
                        &predicates,
                        scopes,
                        return_move_type,
                    );
                    self.cleanup_pop_flow_scope(scopes, &mut flow, statement.span);
                    flow
                } else {
                    self.check_while_with_finally(statement, &[], scopes, return_move_type)
                }
            }
            Stmt::DoWhile(statement) => {
                if let Some(finally) = &statement.finally {
                    let mut attached = statement.clone();
                    attached.finally = None;
                    let flow =
                        self.check_statement(&Stmt::DoWhile(attached), scopes, return_move_type);
                    return self.apply_finally_to_flow(
                        &finally.block,
                        scopes,
                        flow,
                        return_move_type,
                    );
                }
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_block(&statement.body, &mut body, return_move_type, true);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                for repeat in &mut body_flow.backedges {
                    self.use_expr(&statement.condition, repeat, UseMode::Read);
                    self.cleanup_release_temporaries(repeat, statement.condition.span());
                }
                self.check_second_iteration(
                    &statement.body,
                    &body_flow.backedges,
                    return_move_type,
                );
                let condition = constant_bool(&statement.condition);
                let mut exits = body_flow.breaks;
                let returns = body_flow.returns;
                if condition != Some(true) {
                    exits.extend(body_flow.backedges);
                }
                if exits.is_empty() {
                    Flow {
                        returns,
                        ..Flow::stops()
                    }
                } else {
                    merge_reachable_states(scopes, &exits);
                    Flow {
                        returns,
                        ..Flow::fallthrough()
                    }
                }
            }
            Stmt::For(statement) => {
                scopes.push();
                if let Some(initializer) = &statement.initializer {
                    match initializer {
                        ast::ForInitializer::VarDecl(decl) => {
                            let _ = self.check_statement(
                                &Stmt::VarDecl(decl.clone()),
                                scopes,
                                return_move_type,
                            );
                        }
                        ast::ForInitializer::Assignment(assignment) => {
                            let _ = self.check_statement(
                                &Stmt::Assignment(assignment.clone()),
                                scopes,
                                return_move_type,
                            );
                        }
                    }
                }
                if let Some(condition) = &statement.condition {
                    self.use_expr(condition, scopes, UseMode::Read);
                    self.cleanup_release_temporaries(scopes, condition.span());
                    if constant_bool(condition) == Some(false) {
                        self.cleanup_pop_scope(scopes, statement.span, cleanup::Cause::ScopeExit);
                        return Flow::fallthrough();
                    }
                }
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_block(&statement.body, &mut body, return_move_type, true);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                for repeat in &mut body_flow.backedges {
                    self.check_for_tail(statement, repeat, return_move_type);
                }
                self.check_for_second_iteration(statement, &body_flow.backedges, return_move_type);
                let returns = body_flow.returns;
                let mut exits = body_flow.breaks;
                if statement.condition.as_ref().and_then(constant_bool) != Some(true)
                    && statement.condition.is_some()
                {
                    exits.push(before);
                    exits.extend(body_flow.backedges);
                }
                let falls_through = !exits.is_empty();
                if falls_through {
                    merge_reachable_states(scopes, &exits);
                }
                let mut flow = Flow {
                    falls_through,
                    returns,
                    ..Flow::stops()
                };
                self.cleanup_pop_flow_scope(scopes, &mut flow, statement.span);
                flow
            }
            Stmt::Foreach(statement) => {
                self.use_expr(&statement.iterable, scopes, UseMode::Read);
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_foreach_iteration(statement, &mut body, return_move_type);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                self.check_foreach_second_iteration(
                    statement,
                    &body_flow.backedges,
                    return_move_type,
                );
                let returns = body_flow.returns;
                let mut exits = body_flow.backedges;
                exits.extend(body_flow.breaks);
                merge_loop_exit(scopes, &before, &exits);
                Flow {
                    returns,
                    ..Flow::fallthrough()
                }
            }
            Stmt::Increment(increment) => {
                if self.use_property_update(&increment.target, None, scopes) {
                    return Flow::fallthrough();
                }
                for root in self.borrow_root_keys(&increment.target, scopes) {
                    self.check_live_retained_conflict(
                        &root,
                        UseMode::Write,
                        increment.target.span(),
                        scopes,
                    );
                }
                self.use_expr(&increment.target, scopes, UseMode::Read);
                Flow::fallthrough()
            }
            Stmt::Throw(statement) => {
                let diagnostics_before = self.diagnostics.len();
                self.use_expr(&statement.expr, scopes, UseMode::Give);
                for diagnostic in &mut self.diagnostics[diagnostics_before..] {
                    if diagnostic.code == "E0474" {
                        let help = "throw transfers ownership; use an owned Error value or accept the parameter with `take`".to_string();
                        diagnostic.title = "Throw Requires Ownership".to_string();
                        diagnostic.helps.clear();
                        diagnostic.helps.push(help.clone());
                        diagnostic.help = Some(help);
                    }
                }
                let transferred = self.cleanup_take_expr(&statement.expr, scopes);
                self.record_exceptional_effects_with_values(
                    statement.span,
                    crate::checked_effects::effects_at(self.checked_effect_sites, statement.span),
                    scopes,
                    Some(&transferred),
                );
                Flow::stops()
            }
            Stmt::Try(statement) => {
                let before = scopes.clone();
                let mut protected_scopes = before.clone();
                self.push_exception_scope(&before);
                let mut protected_flow = self.check_block(
                    &statement.body,
                    &mut protected_scopes,
                    return_move_type,
                    true,
                );
                let mut unmatched_exits = self.pop_exception_scope();

                let mut fallthrough_states = Vec::new();
                if protected_flow.falls_through {
                    fallthrough_states.push(protected_scopes.clone());
                }
                self.push_exception_scope(&before);
                for catch in &statement.catches {
                    if !self.catch_error_types.contains_key(&catch.span) {
                        continue;
                    }
                    let mut caught_states = Vec::new();
                    let mut caught_values = HashSet::new();
                    let mut remaining = Vec::new();
                    for exit in unmatched_exits {
                        let coverage = crate::checked_effects::catch_coverage(
                            self.catch_coverage,
                            catch.span,
                            &exit.effect,
                        );
                        if coverage != crate::checked_effects::CatchCoverage::None {
                            caught_states.push(exit.scopes.clone());
                            caught_values.extend(&exit.values);
                        }
                        if coverage != crate::checked_effects::CatchCoverage::Complete {
                            remaining.push(exit);
                        }
                    }
                    unmatched_exits = remaining;
                    if caught_states.is_empty() {
                        continue;
                    }

                    let mut catch_scopes = caught_states[0].clone();
                    merge_reachable_states(&mut catch_scopes, &caught_states);
                    catch_scopes.push();
                    if let Some(binding) = &catch.binding {
                        catch_scopes.declare(
                            binding.name.clone(),
                            Binding {
                                id: self.next_binding_id(),
                                canonical_id: self.canonical_binding_id(binding.span),
                                class: type_ref_class_name(
                                    &catch.ty,
                                    &self.classes,
                                    self.receiver_class.as_deref(),
                                ),
                                collection: None,
                                mixed: false,
                                borrowed_place: false,
                                borrow_root: None,
                                borrow_parents: Vec::new(),
                                writeback_homes: Vec::new(),
                                writable: false,
                                state: State::Owned,
                                function_type: None,
                                retained_value: None,
                                scope_depth: catch_scopes.lexical_depth(),
                            },
                        );
                        if let Some(id) = self.canonical_binding_id(binding.span) {
                            catch_scopes.1.put(
                                cleanup::Slot::Binding(id),
                                caught_values,
                                catch_scopes.lexical_depth(),
                            );
                        }
                    } else {
                        catch_scopes.1.put(
                            cleanup::Slot::CaughtError(catch.span),
                            caught_values,
                            catch_scopes.lexical_depth(),
                        );
                    }
                    let mut catch_flow =
                        self.check_block(&catch.body, &mut catch_scopes, return_move_type, false);
                    self.cleanup_pop_flow_scope(&mut catch_scopes, &mut catch_flow, catch.span);
                    if catch_flow.falls_through {
                        fallthrough_states.push(catch_scopes);
                    }
                    protected_flow.backedges.append(&mut catch_flow.backedges);
                    protected_flow.breaks.append(&mut catch_flow.breaks);
                    protected_flow.returns.append(&mut catch_flow.returns);
                    protected_flow.yields.append(&mut catch_flow.yields);
                }
                let mut propagating_exits = unmatched_exits;
                propagating_exits.extend(self.pop_exception_scope());

                if fallthrough_states.is_empty() {
                    *scopes = before;
                    protected_flow.falls_through = false;
                } else {
                    merge_reachable_states(scopes, &fallthrough_states);
                    protected_flow.falls_through = true;
                }

                if let Some(finally) = &statement.finally {
                    let mut overrides = self.apply_finally_to_exceptional_exits(
                        &finally.body,
                        &mut propagating_exits,
                        return_move_type,
                    );
                    self.propagate_exceptional_exits(propagating_exits);
                    let mut finalized = self.apply_finally_to_flow(
                        &finally.body,
                        scopes,
                        protected_flow,
                        return_move_type,
                    );
                    finalized.backedges.append(&mut overrides.backedges);
                    finalized.breaks.append(&mut overrides.breaks);
                    finalized.returns.append(&mut overrides.returns);
                    finalized.yields.append(&mut overrides.yields);
                    finalized
                } else {
                    self.propagate_exceptional_exits(propagating_exits);
                    protected_flow
                }
            }
            Stmt::Break { .. } => Flow::breaks(scopes),
            Stmt::Continue { .. } => Flow {
                falls_through: false,
                backedges: vec![scopes.clone()],
                breaks: Vec::new(),
                returns: Vec::new(),
                yields: Vec::new(),
            },
        }
    }

    fn apply_finally_to_flow(
        &mut self,
        finally: &ast::Block,
        scopes: &mut Scopes,
        flow: Flow,
        return_move_type: bool,
    ) -> Flow {
        let mut result = if flow.falls_through {
            self.check_block(finally, scopes, return_move_type, true)
        } else {
            Flow::stops()
        };
        self.cleanup_superseded_control_exit(&mut result, finally.span);
        for (kind, exits) in [
            (0, flow.backedges),
            (1, flow.breaks),
            (2, flow.returns),
            (3, flow.yields),
        ] {
            for mut state in exits {
                let mut finalized = self.check_block(finally, &mut state, return_move_type, true);
                self.cleanup_superseded_control_exit(&mut finalized, finally.span);
                if finalized.falls_through {
                    match kind {
                        0 => result.backedges.push(state),
                        1 => result.breaks.push(state),
                        2 => result.returns.push(state),
                        _ => result.yields.push(state),
                    }
                }
                result.backedges.append(&mut finalized.backedges);
                result.breaks.append(&mut finalized.breaks);
                result.returns.append(&mut finalized.returns);
                result.yields.append(&mut finalized.yields);
            }
        }
        result
    }

    fn check_given_setup<'a>(
        &mut self,
        given: &'a ast::GivenPrelude,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Vec<&'a Expr> {
        let predicate_indices = self
            .given_preludes
            .get(&given.span)
            .map(|info| {
                info.predicate_statement_indices
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let mut predicates = Vec::new();
        for (index, statement) in given.block.statements.iter().enumerate() {
            if predicate_indices.contains(&index) {
                let Stmt::Expr { expr, .. } = statement else {
                    unreachable!("checked given predicate plan identifies an expression")
                };
                predicates.push(expr);
            }
            let _ = self.check_statement(statement, scopes, return_move_type);
            self.cleanup_release_temporaries(scopes, cleanup::statement_span(statement));
        }
        predicates
    }

    fn check_while_statement(
        &mut self,
        statement: &ast::WhileStmt,
        predicates: &[&Expr],
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Flow {
        self.use_expr(&statement.condition, scopes, UseMode::Read);
        self.cleanup_release_temporaries(scopes, statement.condition.span());
        if predicates
            .iter()
            .any(|predicate| constant_bool(predicate) == Some(false))
            || constant_bool(&statement.condition) == Some(false)
        {
            return Flow::fallthrough();
        }
        let before = scopes.clone();
        let mut body = before.clone();
        let mut body_flow = self.check_block(&statement.body, &mut body, return_move_type, true);
        if body_flow.falls_through {
            body_flow.backedges.push(body);
        }
        for repeat in &mut body_flow.backedges {
            for predicate in predicates {
                self.use_expr(predicate, repeat, UseMode::Read);
            }
            self.use_expr(&statement.condition, repeat, UseMode::Read);
            self.cleanup_release_temporaries(repeat, statement.condition.span());
        }
        self.check_second_iteration(&statement.body, &body_flow.backedges, return_move_type);
        let returns = body_flow.returns;
        let mut exits = body_flow.breaks;
        if constant_bool(&statement.condition) != Some(true) {
            exits.push(before);
            exits.extend(body_flow.backedges);
        }
        let falls_through = !exits.is_empty();
        if falls_through {
            merge_reachable_states(scopes, &exits);
        }
        Flow {
            falls_through,
            returns,
            ..Flow::stops()
        }
    }

    fn check_while_with_finally(
        &mut self,
        statement: &ast::WhileStmt,
        predicates: &[&Expr],
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Flow {
        let Some(finally) = &statement.finally else {
            return self.check_while_statement(statement, predicates, scopes, return_move_type);
        };
        let mut attached = statement.clone();
        attached.finally = None;
        let flow = self.check_while_statement(&attached, predicates, scopes, return_move_type);
        self.apply_finally_to_flow(&finally.block, scopes, flow, return_move_type)
    }

    fn check_foreach_iteration(
        &mut self,
        statement: &ast::ForeachStmt,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Flow {
        let borrow_depth = self.active_borrows.len();
        self.activate_place_borrow(&statement.iterable, UseMode::Read, scopes);
        scopes.push();
        if let Some(first_binding) = &statement.first_binding {
            self.declare_foreach_binding(first_binding, scopes);
        }
        self.declare_foreach_binding(&statement.value_binding, scopes);
        let mut flow = self.check_block(&statement.body, scopes, return_move_type, false);
        self.cleanup_pop_flow_scope(scopes, &mut flow, statement.span);
        self.active_borrows.truncate(borrow_depth);
        flow
    }

    fn declare_foreach_binding(&mut self, binding: &ast::ForeachBinding, scopes: &mut Scopes) {
        let Some(ty) = &binding.ty else {
            return;
        };
        let canonical_id = self.canonical_binding_id(binding.span);
        scopes.declare(
            binding.name.clone(),
            Binding {
                id: self.next_binding_id(),
                canonical_id,
                class: type_ref_class_name(ty, &self.classes, self.receiver_class.as_deref()),
                collection: type_ref_collection_info(
                    ty,
                    &self.classes,
                    &self.move_nominal_names,
                    self.receiver_class.as_deref(),
                    &self.current_type_params,
                    &[],
                ),
                mixed: ty.name == "mixed",
                borrowed_place: true,
                borrow_root: None,
                borrow_parents: Vec::new(),
                writeback_homes: Vec::new(),
                writable: binding.writable,
                state: State::Borrowed,
                function_type: self.canonical_function_type(canonical_id),
                retained_value: self.parameter_function_value(
                    canonical_id,
                    if binding.writable {
                        BindingOwnership::WritableBorrow
                    } else {
                        BindingOwnership::ReadonlyBorrow
                    },
                    None,
                ),
                scope_depth: scopes.lexical_depth(),
            },
        );
    }

    fn check_foreach_second_iteration(
        &mut self,
        statement: &ast::ForeachStmt,
        entries: &[Scopes],
        return_move_type: bool,
    ) {
        for entry in entries {
            let diagnostics_before = self.diagnostics.len();
            let mut second_iteration = entry.clone();
            let _ =
                self.check_foreach_iteration(statement, &mut second_iteration, return_move_type);
            self.deduplicate_diagnostics_from(diagnostics_before);
        }
    }

    fn check_second_iteration(
        &mut self,
        body: &ast::Block,
        backedges: &[Scopes],
        return_move_type: bool,
    ) {
        for backedge in backedges {
            let diagnostics_before = self.diagnostics.len();
            let mut second_iteration = backedge.clone();
            let _ = self.check_block(body, &mut second_iteration, return_move_type, true);
            self.deduplicate_diagnostics_from(diagnostics_before);
        }
    }

    fn check_for_second_iteration(
        &mut self,
        statement: &ast::ForStmt,
        entries: &[Scopes],
        return_move_type: bool,
    ) {
        for entry in entries {
            let diagnostics_before = self.diagnostics.len();
            let mut second_iteration = entry.clone();
            let mut flow = self.check_block(
                &statement.body,
                &mut second_iteration,
                return_move_type,
                true,
            );
            if flow.falls_through {
                flow.backedges.push(second_iteration);
            }
            for backedge in &mut flow.backedges {
                self.check_for_tail(statement, backedge, return_move_type);
            }
            self.deduplicate_diagnostics_from(diagnostics_before);
        }
    }

    fn check_for_tail(
        &mut self,
        statement: &ast::ForStmt,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) {
        if let Some(increment) = &statement.increment {
            match increment {
                ast::ForIncrement::Assignment(assignment) => {
                    let _ = self.check_statement(
                        &Stmt::Assignment((**assignment).clone()),
                        scopes,
                        return_move_type,
                    );
                }
                ast::ForIncrement::Increment(increment) => {
                    self.use_expr(&increment.target, scopes, UseMode::Read);
                }
            }
        }
        if let Some(condition) = &statement.condition {
            self.use_expr(condition, scopes, UseMode::Read);
            self.cleanup_release_temporaries(scopes, condition.span());
        }
    }

    fn deduplicate_diagnostics_from(&mut self, start: usize) {
        let mut additions = self.diagnostics.split_off(start);
        additions.retain(|candidate| {
            !self.diagnostics.iter().any(|existing| {
                existing.code == candidate.code
                    && existing.message == candidate.message
                    && existing.span == candidate.span
            })
        });
        self.diagnostics.extend(additions);
    }

    fn use_assignment_operands(&mut self, target: &Expr, value: &Expr, scopes: &mut Scopes) {
        self.use_assignment_operands_with_mode(target, value, scopes, UseMode::Read);
    }

    fn use_assignment_operands_with_mode(
        &mut self,
        target: &Expr,
        value: &Expr,
        scopes: &mut Scopes,
        value_mode: UseMode,
    ) {
        self.use_expr(target, scopes, UseMode::Write);
        let mut ungrouped_target = target;
        while let Expr::Grouped { expr, .. } = ungrouped_target {
            ungrouped_target = expr;
        }
        let tracked_target = matches!(
            ungrouped_target,
            Expr::PropertyAccess { .. } | Expr::StaticMember { .. } | Expr::Index { .. }
        );
        let assignment_roots = if tracked_target {
            self.borrow_root_keys(target, scopes)
        } else {
            Vec::new()
        };
        let inserted = assignment_roots
            .into_iter()
            .filter(|root| self.active_assignment_writes.insert(root.clone()))
            .collect::<Vec<_>>();
        let assignment_target = tracked_target
            .then(|| self.assignment_place_key(target, scopes))
            .flatten();
        let target_inserted = assignment_target
            .as_ref()
            .is_some_and(|target| self.active_assignment_targets.insert(target.clone()));
        let already_checked = self.retained_value_from_expr(value, scopes).is_some();
        let retained = self.evaluate_retained_value(value, scopes, value_mode);
        if tracked_target && !already_checked {
            if let Some(retained) = retained {
                if let Expr::Index { collection, .. } = ungrouped_target {
                    self.validate_collection_destination(
                        collection,
                        &retained,
                        value.span(),
                        scopes,
                    );
                } else {
                    self.validate_owned_function_storage(
                        &retained,
                        value.span(),
                        "an owned aggregate",
                    );
                }
            }
        }
        if target_inserted {
            self.active_assignment_targets
                .remove(assignment_target.as_deref().expect("inserted target"));
        }
        for root in inserted {
            self.active_assignment_writes.remove(&root);
        }
    }

    fn acquire_closure(&mut self, closure: &ast::ClosureExpression, scopes: &mut Scopes) {
        let closure_id = ClosureId::from_span(closure.span);
        let Some(semantic) = self.closures.get(&closure_id).cloned() else {
            return;
        };
        let cause = format!("closure:{}:{}:ownership", closure_id.start, closure_id.end);
        let mut acquisitions = Vec::new();
        let mut leases = Vec::new();
        let mut move_sources = Vec::new();
        let mut invalid = false;

        for capture in &semantic.captures {
            let declaration = self
                .binding_resolution
                .declarations_by_id
                .get(&capture.source_binding_id);
            let source_span = declaration.and_then(|declaration| declaration.span);
            let source_function_value = scopes
                .get_by_canonical(capture.source_binding_id)
                .and_then(|(_, binding)| binding.retained_value.clone());
            let immutable_null_source = scopes
                .get_by_canonical(capture.source_binding_id)
                .is_some_and(|(_, binding)| {
                    binding.borrow_root == Some(BorrowSourceRoot::ImmutableNull)
                });
            let source_is_move =
                resolved_type_is_move_type(&capture.source_type, &self.move_enum_names)
                    || resolved_type_requires_conservative_move(&capture.source_type);
            let (root, root_keys, source_depth) = if declaration.is_some_and(|declaration| {
                declaration.kind == BindingKind::MethodReceiver
                    && declaration.ownership != BindingOwnership::Owned
            }) {
                (BorrowRoot::Receiver, vec!["$this".into()], 0)
            } else if let Some((name, binding)) = scopes.get_by_canonical(capture.source_binding_id)
            {
                let root = declaration
                    .and_then(|declaration| match declaration.owner {
                        crate::symbols::LexicalOwner::Closure(owner)
                            if declaration.kind != BindingKind::ClosureParameter =>
                        {
                            Some(BorrowRoot::EnclosingEnvironment(owner))
                        }
                        _ => None,
                    })
                    .unwrap_or(BorrowRoot::Binding(capture.source_binding_id));
                (root, binding_roots(binding, name), binding.scope_depth)
            } else {
                (
                    BorrowRoot::Binding(capture.source_binding_id),
                    vec![format!("binding:{}", capture.source_binding_id.0).into()],
                    scopes.lexical_depth(),
                )
            };

            let kind = match capture.mode {
                ast::ClosureCaptureMode::Readonly => CaptureAcquisitionKind::ReadonlyLease,
                ast::ClosureCaptureMode::Writable => CaptureAcquisitionKind::WritableLease,
                ast::ClosureCaptureMode::Take if source_is_move => {
                    CaptureAcquisitionKind::MoveIntoEnvironment
                }
                ast::ClosureCaptureMode::Take => CaptureAcquisitionKind::CopyIntoEnvironment,
            };

            let access = match kind {
                CaptureAcquisitionKind::ReadonlyLease if immutable_null_source => None,
                CaptureAcquisitionKind::ReadonlyLease => Some(BorrowAccess::Readonly),
                CaptureAcquisitionKind::WritableLease => Some(BorrowAccess::Writable),
                CaptureAcquisitionKind::CopyIntoEnvironment
                | CaptureAcquisitionKind::MoveIntoEnvironment => None,
            };
            let source_origins = scopes
                .get_by_canonical(capture.source_binding_id)
                .and_then(|(_, binding)| binding.borrow_root.clone())
                .map(|source| {
                    self.retained_loans_at_source(
                        Some(source),
                        capture.declaration_span,
                        scopes,
                        access.unwrap_or(BorrowAccess::Readonly),
                    )
                    .into_iter()
                    .map(|loan| (loan.root, loan.root_key, loan.source_depth))
                    .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| {
                    root_keys
                        .iter()
                        .cloned()
                        .map(|key| (root.clone(), key, source_depth))
                        .collect()
                });
            if let Some(access) = access {
                for (source_root, root_key, source_depth) in &source_origins {
                    if let Some((_, conflict)) = scopes.conflicting_retained_loan(|lease| {
                        lease.places().any(|(root, retained_access)| {
                            root.overlaps(root_key)
                                && (access == BorrowAccess::Writable
                                    || retained_access == BorrowAccess::Writable)
                        })
                    }) {
                        invalid = true;
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0654",
                                "closure capture conflicts with an earlier live access",
                                capture.declaration_span,
                            )
                            .with_title("Closure Capture Conflicts With Live Access")
                            .with_cause(cause.clone())
                            .with_primary_label("Conflicting Capture Starts Here")
                            .with_related(
                                conflict.capture_span,
                                "Earlier Capture Remains Usable Here",
                            )
                            .with_help(
                                "finish using the earlier closure before creating this capture",
                            ),
                        );
                    }
                    if let Some(conflict) = self.active_borrows.iter().find(|borrow| {
                        borrow.root.overlaps(root_key)
                            && (access == BorrowAccess::Writable || borrow.mode == UseMode::Write)
                    }) {
                        invalid = true;
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0654",
                                "closure capture conflicts with an earlier live access",
                                capture.declaration_span,
                            )
                            .with_title("Closure Capture Conflicts With Live Access")
                            .with_cause(cause.clone())
                            .with_related(conflict.span, "Earlier Access Starts Here")
                            .with_help("finish the earlier access before creating this closure"),
                        );
                    }
                    {
                        let through = scopes
                            .get_by_canonical(capture.source_binding_id)
                            .map(|(_, binding)| binding.id)
                            .into_iter()
                            .collect::<Vec<_>>();
                        let mode = if access == BorrowAccess::Writable {
                            UseMode::Write
                        } else {
                            UseMode::Read
                        };
                        if let Some(alias) = scopes.borrowed_from(root_key, &through, mode) {
                            invalid = true;
                            self.diagnostics.push(
                            Diagnostic::new(
                                "E0654",
                                format!("closure capture conflicts with live borrowed binding `${alias}`"),
                                capture.declaration_span,
                            )
                            .with_title("Closure Capture Conflicts With Live Access")
                            .with_cause(cause.clone())
                            .with_help("finish using the readonly binding before creating this closure"),
                        );
                        }
                    }
                    leases.push(RetainedLoan {
                        root: source_root.clone(),
                        root_key: root_key.clone(),
                        access,
                        capture_span: capture.declaration_span,
                        source_depth: *source_depth,
                        inherited: false,
                        lifetime: RetainedLifetime::LastUse,
                        reborrows: Vec::new(),
                        pending_capture: None,
                    });
                }
            }

            if kind == CaptureAcquisitionKind::MoveIntoEnvironment {
                match scopes.get_by_canonical(capture.source_binding_id) {
                    Some((name, binding))
                        if matches!(binding.state, State::Borrowed | State::BorrowedOrOwned) =>
                    {
                        invalid = true;
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0645",
                                format!(
                                    "borrowed move value `${}` cannot be taken into a closure",
                                    name
                                ),
                                capture.declaration_span,
                            )
                            .with_title("Taking Capture Requires Ownership")
                            .with_cause(cause.clone())
                            .with_help(
                                "Take an owned local, or capture this borrowed value readonly.",
                            ),
                        );
                    }
                    Some((_, binding))
                        if matches!(
                            binding.state,
                            State::Given { .. } | State::MaybeGiven { .. }
                        ) =>
                    {
                        invalid = true;
                        let at = match binding.state {
                            State::Given { at } | State::MaybeGiven { at } => at,
                            _ => capture.declaration_span,
                        };
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0655",
                                "captured source is no longer available for ownership transfer",
                                capture.declaration_span,
                            )
                            .with_title("Captured Value Was Already Moved")
                            .with_cause(cause.clone())
                            .with_related(at, "Value Moved Here")
                            .with_help("move the value into exactly one closure"),
                        );
                    }
                    Some(_) => move_sources.push(capture.source_binding_id),
                    None => {}
                }
                if let Some((_, conflict)) = scopes.conflicting_retained_loan(|lease| {
                    lease
                        .places()
                        .any(|(root, _)| root_keys.iter().any(|key| root.overlaps(key)))
                }) {
                    invalid = true;
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0654",
                            "taking capture conflicts with an earlier live access",
                            capture.declaration_span,
                        )
                        .with_title("Taking Capture Conflicts With Live Access")
                        .with_cause(cause.clone())
                        .with_related(conflict.capture_span, "Earlier Capture Remains Usable Here")
                        .with_help(
                            "finish using the earlier closure before transferring ownership",
                        ),
                    );
                }
                if let Some(conflict) = self
                    .active_borrows
                    .iter()
                    .find(|borrow| root_keys.iter().any(|key| borrow.root.overlaps(key)))
                {
                    invalid = true;
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0654",
                            "taking capture conflicts with an earlier live access",
                            capture.declaration_span,
                        )
                        .with_title("Taking Capture Conflicts With Live Access")
                        .with_cause(cause.clone())
                        .with_related(conflict.span, "Earlier Access Starts Here")
                        .with_help("finish the earlier access before transferring ownership"),
                    );
                }
            }

            if matches!(
                kind,
                CaptureAcquisitionKind::MoveIntoEnvironment
                    | CaptureAcquisitionKind::CopyIntoEnvironment
            ) {
                if let Some(retained_value) = source_function_value.as_ref() {
                    retained::append_loans(&mut leases, &retained_value.leases);
                }
            }

            acquisitions.push(CaptureAcquisition {
                environment_binding_id: capture.environment_binding_id,
                source_binding_id: capture.source_binding_id,
                kind,
                source_type: capture.source_type.clone(),
                roots: if access.is_some() {
                    let mut roots = Vec::new();
                    for (root, _, _) in &source_origins {
                        if !roots.contains(root) {
                            roots.push(root.clone());
                        }
                    }
                    roots
                } else {
                    source_function_value
                        .as_ref()
                        .map(|value| provenance_roots(&value.provenance))
                        .unwrap_or_default()
                },
                source_span,
                capture_span: capture.declaration_span,
            });
        }

        if invalid {
            self.closure_values.remove(&closure_id);
            return;
        }
        let mut cleanup_contents = HashSet::new();
        for source in move_sources {
            cleanup_contents.extend(scopes.1.take(&cleanup::Slot::Binding(source)));
            if let Some(binding) = scopes.get_mut_by_canonical(source) {
                binding.state = State::Given {
                    at: semantic.execution_boundary_span,
                };
            }
        }
        if let Some(owner) = self.cleanup_owner {
            let value = self.cleanup_analysis.acquire(
                owner,
                cleanup::Source::Expression(closure.span),
                semantic.function_type.clone(),
                cleanup_contents,
            );
            scopes.1.put(
                cleanup::Slot::Temporary(closure.span),
                HashSet::from([value]),
                scopes.lexical_depth(),
            );
        }
        let mut roots = leases
            .iter()
            .map(|lease| lease.root.clone())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        let provenance = if roots.is_empty() {
            ValueProvenance::Owned
        } else {
            ValueProvenance::BorrowBound(roots)
        };
        let mut release_order = (0..acquisitions.len()).collect::<Vec<_>>();
        release_order.reverse();
        let invocation_consumption =
            if semantic.inferred_invocation_mode == FunctionInvocationMode::Once {
                InvocationConsumption::Once
            } else {
                InvocationConsumption::Repeatable
            };
        let info = ClosureOwnershipInfo {
            closure_id,
            provenance: provenance.clone(),
            acquisitions,
            release_order,
            escape: ClosureEscapeClassification::Local,
            invocation_consumption,
        };
        self.closure_ownership.insert(closure_id, info);
        self.closure_values.insert(
            closure_id,
            RetainedValueState {
                kind: RetainedValueKind::Closure,
                closure_id: Some(closure_id),
                provenance,
                leases,
                nonescaping_parameter: false,
                take_parameter_insertion: None,
                storage: owned_storage::Contents::default(),
                enum_payloads: None,
            },
        );
    }

    fn retained_value_from_expr(&self, expr: &Expr, scopes: &Scopes) -> Option<RetainedValueState> {
        if self.flow_fact(expr) == Some(&Fact::Null) {
            return None;
        }
        let value = match expr {
            Expr::When(_) | Expr::Match { .. } => {
                self.retained.expression_values.get(&expr.span()).cloned()
            }
            Expr::StaticCall { span, .. } if self.enum_case_constructions.contains_key(span) => {
                self.retained.expression_values.get(span).cloned()
            }
            Expr::Closure(closure) => self
                .closure_values
                .get(&ClosureId::from_span(closure.span))
                .cloned(),
            Expr::Variable { name, .. } => scopes
                .get(name)
                .and_then(|binding| binding.retained_value.clone()),
            Expr::Grouped { expr, .. } => self.retained_value_from_expr(expr, scopes),
            Expr::Array { elements, .. } => {
                let mut stored: Option<RetainedValueState> = None;
                for element in elements {
                    for input in element.key.iter().chain(std::iter::once(&element.value)) {
                        if let Some(value) = self.retained_value_from_expr(input, scopes) {
                            let stored =
                                stored.get_or_insert_with(|| retained::iterator_value(Vec::new()));
                            stored.storage.merge_from(
                                &value.storage,
                                &stored.leases,
                                &value.leases,
                            );
                            stored.refresh_stored_loans();
                        }
                    }
                }
                stored
            }
            Expr::ArrayRepeat { value, .. } => {
                self.retained_value_from_expr(value, scopes).map(|value| {
                    let mut stored = retained::iterator_value(Vec::new());
                    stored
                        .storage
                        .merge_from(&value.storage, &[], &value.leases);
                    stored.refresh_stored_loans();
                    stored
                })
            }
            Expr::Null { .. } => None,
            _ => self
                .resolved_type(expr)
                .and_then(non_null_function_type)
                .map(|_| {
                    if let Some(mut value) = self.iterator_value_from_expr(expr, scopes) {
                        value.kind = RetainedValueKind::Closure;
                        return value;
                    }
                    let loans = if self.expr_has_return_contract(expr, scopes, None) {
                        let access = if self.function_result_borrow_is_writable(expr, scopes) {
                            BorrowAccess::Writable
                        } else {
                            BorrowAccess::Readonly
                        };
                        self.retained_source_loans(expr, scopes, access)
                            .into_iter()
                            .map(|mut loan| {
                                loan.lifetime =
                                    self.function_result_retained_lifetime(expr, scopes);
                                loan
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    let provenance = retained::loan_provenance(&loans);
                    RetainedValueState {
                        kind: RetainedValueKind::Closure,
                        closure_id: None,
                        provenance,
                        leases: loans,
                        nonescaping_parameter: false,
                        take_parameter_insertion: None,
                        storage: owned_storage::Contents::default(),
                        enum_payloads: None,
                    }
                })
                .or_else(|| self.iterator_value_from_expr(expr, scopes)),
        };
        value.or_else(|| self.unknown_storage_selector(expr, scopes))
    }

    fn function_result_borrow_is_writable(&self, expr: &Expr, scopes: &Scopes) -> bool {
        if let Some(sources) = self
            .conditional_borrow_sources
            .get(&ungroup_expr(expr).span())
        {
            return sources.origins.writable;
        }
        if let Some((signature, _, _)) = self.retained_call(expr, scopes) {
            return signature
                .return_borrow
                .is_some_and(|borrow| borrow.writable);
        }
        match ungroup_expr(expr) {
            Expr::CallableCall { span, .. } => self
                .callable_value_calls
                .get(span)
                .and_then(|call| non_null_function_type(&call.function_type))
                .and_then(|function| function.return_borrow)
                .is_some_and(|borrow| borrow.writable),
            _ => false,
        }
    }

    fn function_result_retained_lifetime(&self, expr: &Expr, scopes: &Scopes) -> RetainedLifetime {
        if let Some(sources) = self
            .conditional_borrow_sources
            .get(&ungroup_expr(expr).span())
        {
            return sources.origins.lifetime;
        }
        self.retained_call(expr, scopes)
            .and_then(|(signature, _, _)| signature.return_borrow.map(|borrow| borrow.lifetime))
            .or_else(|| {
                self.callable_value_calls
                    .get(&ungroup_expr(expr).span())
                    .and_then(|call| non_null_function_type(&call.function_type))
                    .and_then(|function| function.return_borrow)
                    .map(|borrow| borrow.lifetime)
            })
            .unwrap_or(RetainedLifetime::LastUse)
    }

    fn call_borrow_source_expr<'a>(
        &self,
        borrow: ReturnBorrow,
        receiver: Option<&'a Expr>,
        signature: &Signature,
        args: &'a [Argument],
    ) -> Option<&'a Expr> {
        match borrow.source {
            BorrowSource::Receiver => receiver,
            BorrowSource::Parameter(index) => {
                let bound = signature.bind_arguments(args);
                let argument = bound.param_to_arg.get(index).copied().flatten()?;
                args.get(argument).map(|argument| &argument.value)
            }
        }
    }

    fn prepare_retained_value(
        &mut self,
        expr: &Expr,
        scopes: &mut Scopes,
    ) -> Option<RetainedValueState> {
        if matches!(ungroup_expr(expr), Expr::When(_) | Expr::Match { .. }) {
            self.retained
                .expression_values
                .remove(&ungroup_expr(expr).span());
        }
        if let Expr::Closure(closure) = ungroup_expr(expr) {
            if self
                .prepared_closure_evaluations
                .insert(ClosureId::from_span(closure.span))
            {
                self.acquire_closure(closure, scopes);
            }
        }
        self.retained_value_from_expr(expr, scopes)
    }

    fn evaluate_retained_value(
        &mut self,
        expr: &Expr,
        scopes: &mut Scopes,
        mode: UseMode,
    ) -> Option<RetainedValueState> {
        self.prepare_retained_value(expr, scopes);
        self.use_expr(expr, scopes, mode);
        self.retained_value_from_expr(expr, scopes)
    }

    fn validate_local_function_destination(
        &mut self,
        value: &RetainedValueState,
        destination_depth: usize,
        span: Span,
    ) -> bool {
        let Some(lease) = value
            .leases
            .iter()
            .find(|lease| lease.source_depth > destination_depth)
        else {
            return true;
        };
        if value.kind == RetainedValueKind::SourceCarrier {
            self.diagnostics.push(retained::iterator_escape(
                span,
                lease.capture_span,
                "borrow-bound value cannot remain usable after its source leaves scope",
            ));
            return false;
        }
        let diagnostic = Diagnostic::new(
                "E0658",
                "closure cannot remain usable after its captured value leaves scope",
                span,
            )
            .with_title("Closure Cannot Outlive Captured Value")
            .with_primary_label("Closure Escapes To A Longer-Lived Binding Here")
            .with_related(lease.capture_span, "Captured Value Is Borrowed Here")
            .with_help("keep the closure inside the captured value's scope or capture an owned value with `take`");
        let diagnostic = self.with_taking_capture_fix(diagnostic, value);
        self.diagnostics
            .push(self.with_function_value_cause(diagnostic, value));
        false
    }

    fn validate_returned_function_value(&mut self, value: &RetainedValueState, span: Span) -> bool {
        if value.leases.iter().any(|loan| {
            let BorrowRoot::Binding(binding) = loan.root else {
                return false;
            };
            self.binding_resolution
                .declarations_by_id
                .get(&binding)
                .and_then(|binding| binding.source_type.as_ref())
                .is_some_and(crate::compiler_known_stream::contains_call_scoped_request)
        }) {
            self.diagnostics.push(
                Diagnostic::new("E0772", "a returned callable cannot retain a pending request view", span)
                    .with_title("Pending Request Cannot Escape Its Phase")
                    .with_help("keep the callable inside the preparation or completion call that lends the request"),
            );
            return false;
        }
        if value.kind == RetainedValueKind::SourceCarrier {
            return self.validate_returned_iterator(value, span);
        }
        if value.nonescaping_parameter {
            let diagnostic = Diagnostic::new(
                    "E0657",
                    "nonescaping callback parameter cannot be returned",
                    span,
                )
                .with_title("Nonescaping Callback Cannot Be Retained")
                .with_help("accept the callback through a `take function(...)` parameter when ownership must leave the call");
            let diagnostic = self.with_take_parameter_fix(diagnostic, value);
            self.diagnostics
                .push(self.with_function_value_cause(diagnostic, value));
            return false;
        }
        let ValueProvenance::BorrowBound(roots) = &value.provenance else {
            self.mark_closure_escape(value, ClosureEscapeClassification::Owned);
            return true;
        };
        let mut accepted_root = None;
        for root in roots {
            let accepted = value
                .leases
                .iter()
                .filter(|loan| loan.root == *root)
                .all(|loan| self.parameter_source(loan).is_some());
            if !accepted {
                let diagnostic = Diagnostic::new(
                    "E0658",
                    "closure cannot return while borrowing a local or temporary value",
                    span,
                )
                .with_title("Closure Cannot Outlive Captured Value")
                .with_help(
                    "capture an independently owned value with `take` before returning the closure",
                );
                let diagnostic = self.with_taking_capture_fix(diagnostic, value);
                self.diagnostics
                    .push(self.with_function_value_cause(diagnostic, value));
                return false;
            }
            if accepted_root
                .replace(root)
                .is_some_and(|previous| previous != root)
            {
                self.diagnostics.push(self.with_function_value_cause(
                    Diagnostic::new(
                        "E0659",
                        "returned closure borrows multiple unrelated owners",
                        span,
                    )
                    .with_title("Closure Has Multiple Incompatible Return Roots")
                    .with_help("return a closure tied to one borrowed parameter or to `$this`, or own the captures with `take`"),
                    value,
                ));
                return false;
            }
        }
        self.record_returned_sources(value);
        self.mark_closure_escape(value, ClosureEscapeClassification::ReturnedBorrow);
        true
    }

    fn mark_closure_escape(
        &mut self,
        value: &RetainedValueState,
        escape: ClosureEscapeClassification,
    ) {
        if let Some(info) = value
            .closure_id
            .and_then(|closure_id| self.closure_ownership.get_mut(&closure_id))
        {
            info.escape = escape;
        }
    }

    fn with_function_value_cause(
        &self,
        diagnostic: Diagnostic,
        value: &RetainedValueState,
    ) -> Diagnostic {
        if let Some(closure_id) = value.closure_id {
            diagnostic.with_cause(format!(
                "closure:{}:{}:ownership",
                closure_id.start, closure_id.end
            ))
        } else {
            diagnostic
        }
    }

    fn with_taking_capture_fix(
        &self,
        diagnostic: Diagnostic,
        value: &RetainedValueState,
    ) -> Diagnostic {
        let Some(ownership) = value
            .closure_id
            .and_then(|closure_id| self.closure_ownership.get(&closure_id))
        else {
            return diagnostic;
        };
        if !matches!(ownership.provenance, ValueProvenance::BorrowBound(_)) {
            return diagnostic;
        }

        let mut edits = Vec::new();
        for acquisition in &ownership.acquisitions {
            if acquisition.roots.is_empty() {
                continue;
            }
            if acquisition.kind != CaptureAcquisitionKind::ReadonlyLease {
                return diagnostic;
            }
            let Some(source) = self
                .binding_resolution
                .declarations_by_id
                .get(&acquisition.source_binding_id)
            else {
                return diagnostic;
            };
            // Local ownership is refined by this pass after declaration binding.
            let source_ownership = self
                .binding_ownership
                .get(&acquisition.source_binding_id)
                .copied()
                .unwrap_or(source.ownership);
            if source.kind == BindingKind::MethodReceiver
                || crate::types::ByteRegionKind::from_resolved(&acquisition.source_type).is_some()
                || ((resolved_type_is_move_type(&acquisition.source_type, &self.move_enum_names)
                    || resolved_type_requires_conservative_move(&acquisition.source_type))
                    && source_ownership != BindingOwnership::Owned)
            {
                return diagnostic;
            }
            edits.push(FixEdit {
                source: DiagnosticSource::Current,
                span: Span::new(
                    acquisition.capture_span.start,
                    acquisition.capture_span.start,
                ),
                replacement: "take ".to_string(),
            });
        }
        if edits.is_empty() {
            diagnostic
        } else {
            diagnostic.with_structured_fix(
                "Capture Borrowed Values With Ownership",
                FixApplicability::RequiresReview,
                edits,
            )
        }
    }

    fn with_take_parameter_fix(
        &self,
        diagnostic: Diagnostic,
        value: &RetainedValueState,
    ) -> Diagnostic {
        let Some(insertion) = value.take_parameter_insertion else {
            return diagnostic;
        };
        diagnostic.with_structured_fix(
            "Accept Callback With Ownership",
            FixApplicability::RequiresReview,
            vec![FixEdit {
                source: DiagnosticSource::Current,
                span: insertion,
                replacement: "take ".to_string(),
            }],
        )
    }

    fn validate_owned_function_storage(
        &mut self,
        value: &RetainedValueState,
        span: Span,
        destination: &str,
    ) -> bool {
        if value.kind == RetainedValueKind::SourceCarrier {
            return self.validate_iterator_storage(value, span, destination);
        }
        if value.nonescaping_parameter {
            let diagnostic = Diagnostic::new(
                    "E0657",
                    format!("nonescaping callback parameter cannot be stored in {destination}"),
                    span,
                )
                .with_title("Nonescaping Callback Cannot Be Retained")
                .with_help("accept the callback through a `take function(...)` parameter when retention is intended");
            let diagnostic = self.with_take_parameter_fix(diagnostic, value);
            self.diagnostics
                .push(self.with_function_value_cause(diagnostic, value));
            return false;
        }
        // Opaque owning parameters carry symbolic incoming dependencies, not
        // a newly created borrow of their local carrier. The same storage
        // planner used by other source-carrying values records the required
        // independence at the caller, where the actual dependency set is known.
        if !value.leases.is_empty()
            && value
                .leases
                .iter()
                .all(|loan| loan.inherited && self.parameter_source(loan).is_some())
        {
            return self.validate_iterator_storage(value, span, destination);
        }
        if matches!(value.provenance, ValueProvenance::BorrowBound(_)) {
            let diagnostic = Diagnostic::new(
                    "E0658",
                    format!("borrow-bound closure cannot be stored in {destination}"),
                    span,
                )
                .with_title("Borrow-Bound Closure Cannot Enter Owned Storage")
                .with_help("capture independently owned values with `take`, or keep the closure in a lifetime-safe local");
            let diagnostic = self.with_taking_capture_fix(diagnostic, value);
            self.diagnostics
                .push(self.with_function_value_cause(diagnostic, value));
            return false;
        }
        self.mark_closure_escape(value, ClosureEscapeClassification::Owned);
        true
    }

    /// Source-carrying values move their loans into the constructed value.
    /// Callables still need owned storage: nonescaping callbacks never enter
    /// it, and closure loan transport has its own rule.
    fn validate_transported_storage(
        &mut self,
        value: &RetainedValueState,
        span: Span,
        destination: &str,
    ) -> bool {
        if value.kind == RetainedValueKind::SourceCarrier && !value.nonescaping_parameter {
            self.record_payload_inputs(value);
            return true;
        }
        self.validate_owned_function_storage(value, span, destination)
    }

    fn validate_collection_element(
        &mut self,
        value: &RetainedValueState,
        span: Span,
        destination_depth: usize,
    ) -> bool {
        if value.nonescaping_parameter {
            return self.validate_owned_function_storage(value, span, "an owned collection");
        }
        if !self.validate_local_function_destination(value, destination_depth, span) {
            return false;
        }
        self.mark_closure_escape(value, ClosureEscapeClassification::Owned);
        true
    }

    fn reject_deferred_function_storage(
        &mut self,
        value: &RetainedValueState,
        span: Span,
        destination: &str,
        title: &str,
    ) -> bool {
        if !self.validate_owned_function_storage(value, span, destination) {
            return false;
        }
        if value.kind != RetainedValueKind::Closure {
            return true;
        }
        let diagnostic = Diagnostic::unsupported_stage(
            "E0661",
            format!("function-value storage in {destination} is accepted Doria but is not implemented yet"),
            span,
        )
        .with_title(title)
        .with_help("keep the function value in approved local or owned aggregate storage until this representation lands");
        self.diagnostics
            .push(self.with_function_value_cause(diagnostic, value));
        false
    }

    fn check_live_retained_conflict(
        &mut self,
        root: &BorrowPlace,
        requested: UseMode,
        span: Span,
        scopes: &Scopes,
    ) {
        let Some((kind, lease)) = scopes.conflicting_retained_loan(|lease| {
            lease.places().any(|(key, access)| {
                key.overlaps(root)
                    && (access == BorrowAccess::Writable
                        || matches!(requested, UseMode::Write | UseMode::Give))
            })
        }) else {
            return;
        };
        if kind == RetainedValueKind::SourceCarrier {
            let access = if lease.access == BorrowAccess::Writable {
                "writable"
            } else {
                "readonly"
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0763",
                    format!("source remains in {access} use by a borrow-bound value"),
                    span,
                )
                .with_title("Borrow-Bound Value Keeps Source In Use")
                .with_related(lease.capture_span, "Source Loan Starts Here")
                .with_help(
                    "finish using the borrowed value before conflicting access to its source",
                ),
            );
            return;
        }
        let (title, access) = match lease.access {
            BorrowAccess::Readonly => ("Closure Keeps Value In Readonly Use", "readonly"),
            BorrowAccess::Writable => ("Closure Keeps Value In Writable Use", "writable"),
        };
        self.diagnostics.push(
            Diagnostic::new(
                "E0654",
                format!("value remains in {access} use by a closure"),
                span,
            )
            .with_title(title)
            .with_primary_label("Conflicting Use Occurs Here")
            .with_related(
                lease.capture_span,
                "Closure Capture Starts The Live Use Here",
            )
            .with_help("finish using or moving the closure before this operation"),
        );
    }

    fn use_expr(&mut self, expr: &Expr, scopes: &mut Scopes, mode: UseMode) {
        self.use_expr_with_pending_access(expr, scopes, mode, true);
    }

    fn use_expr_with_pending_access(
        &mut self,
        expr: &Expr,
        scopes: &mut Scopes,
        mode: UseMode,
        check_pending: bool,
    ) {
        if self.resolved_type(expr).is_some_and(|ty| {
            resolved_type_is_move_type(ty, &self.move_enum_names)
                || resolved_type_requires_conservative_move(ty)
        }) && self.borrow_source_root(expr, scopes).is_some()
            && self.borrowed_value_is_writable(expr, scopes)
        {
            self.writable_borrow_sources.insert(expr.span());
        }
        let mode = match self.assertion_callable_invocations.get(&expr.span()) {
            Some(FunctionInvocationMode::Readonly) => UseMode::Read,
            Some(FunctionInvocationMode::Writable) => UseMode::Write,
            Some(FunctionInvocationMode::Once) => UseMode::Give,
            None => mode,
        };
        // An owning destination may box a Copy value without taking its source.
        let mode = if mode == UseMode::Give
            && self.resolved_type(expr).is_some_and(|ty| {
                !resolved_type_requires_conservative_move(ty)
                    && !resolved_type_is_move_type(ty, &self.move_enum_names)
            }) {
            UseMode::Read
        } else {
            mode
        };
        if check_pending {
            self.check_pending_storage_access(expr, mode, scopes);
        }
        let moving_binding = if mode == UseMode::Give {
            match expr {
                Expr::Variable { name, .. } => scopes.get(name).and_then(|binding| {
                    matches!(
                        binding.state,
                        State::Owned | State::BorrowedOrOwned | State::MaybeGiven { .. }
                    )
                    .then_some(binding.canonical_id)
                    .flatten()
                }),
                Expr::This { .. } => scopes.get("this").and_then(|binding| {
                    matches!(binding.state, State::Owned | State::MaybeGiven { .. })
                        .then_some(binding.canonical_id)
                        .flatten()
                }),
                _ => None,
            }
        } else {
            None
        };
        self.use_expr_inner(expr, scopes, mode);
        self.cleanup_expression_result(expr, scopes, moving_binding);
    }

    fn cleanup_expression_result(
        &mut self,
        expr: &Expr,
        scopes: &mut Scopes,
        moving_binding: Option<BindingId>,
    ) {
        let Some(owner) = self.cleanup_owner else {
            return;
        };
        let target = cleanup::Slot::Temporary(expr.span());
        if let Some(binding) = moving_binding {
            let values = scopes.1.take(&cleanup::Slot::Binding(binding));
            scopes.1.put(target, values, scopes.lexical_depth());
            return;
        }
        if let Expr::Grouped { expr: inner, .. } = expr {
            let values = self.cleanup_take_expr(inner, scopes);
            scopes.1.put(target, values, scopes.lexical_depth());
            return;
        }
        if !scopes.1.values(&target).is_empty() {
            return;
        }
        let creates_owner = match expr {
            Expr::New { .. } | Expr::Array { .. } | Expr::ArrayRepeat { .. } | Expr::Closure(_) => {
                true
            }
            Expr::MethodCall {
                object,
                null_safe: true,
                ..
            }
            | Expr::PropertyAccess {
                object,
                null_safe: true,
                ..
            } if self.flow_fact(object) == Some(&Fact::Null) => false,
            Expr::FunctionCall { .. }
            | Expr::MethodCall { .. }
            | Expr::StaticCall { .. }
            | Expr::CallableCall { .. } => !self.expr_returns_borrow(expr, scopes),
            Expr::PropertyAccess { .. } => self.getter_signature(expr).is_some_and(|signature| {
                signature
                    .return_borrow
                    .is_none_or(|borrow| borrow.kind != ReturnBorrowKind::Value)
            }),
            Expr::Binary {
                op: BinaryOp::Coalesce,
                ..
            } => !self.expr_returns_borrow(expr, scopes),
            _ => false,
        };
        if !creates_owner {
            return;
        }
        let Some(ty) = self.resolved_type(expr).cloned() else {
            return;
        };
        if !resolved_type_is_move_type(&ty, &self.move_enum_names)
            && !resolved_type_requires_conservative_move(&ty)
        {
            return;
        }
        let mut contents = HashSet::new();
        match expr {
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.cleanup_analysis.source_flows.insert((
                            cleanup::Source::Expression(key.span()),
                            cleanup::Source::CollectionElement {
                                collection: expr.span(),
                                key: true,
                            },
                        ));
                        contents.extend(self.cleanup_take_expr(key, scopes));
                    }
                    self.cleanup_analysis.source_flows.insert((
                        cleanup::Source::Expression(element.value.span()),
                        cleanup::Source::CollectionElement {
                            collection: expr.span(),
                            key: false,
                        },
                    ));
                    contents.extend(self.cleanup_take_expr(&element.value, scopes));
                }
            }
            Expr::ArrayRepeat { value, .. } => {
                let element = cleanup::Source::CollectionElement {
                    collection: expr.span(),
                    key: false,
                };
                if let Some(element_type) = self.resolved_type(value).cloned() {
                    if resolved_type_is_move_type(&element_type, &self.move_enum_names)
                        || resolved_type_requires_conservative_move(&element_type)
                    {
                        // Fill borrows its seed and owns independently cloned
                        // elements. A fresh seed remains a temporary to release
                        // after construction, even when the array is returned.
                        // The clone's nested state is not the seed's ownership.
                        contents.insert(self.cleanup_analysis.acquire(
                            owner,
                            element,
                            element_type,
                            HashSet::new(),
                        ));
                    } else {
                        self.cleanup_analysis
                            .source_flows
                            .insert((cleanup::Source::Expression(value.span()), element));
                    }
                }
            }
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => {
                contents.extend(self.cleanup_take_expr(left, scopes));
                contents.extend(self.cleanup_take_expr(right, scopes));
                scopes.1.put(target, contents, scopes.lexical_depth());
                return;
            }
            _ => {}
        }
        let value = self.cleanup_analysis.acquire(
            owner,
            cleanup::Source::Expression(expr.span()),
            ty,
            contents,
        );
        scopes
            .1
            .put(target, HashSet::from([value]), scopes.lexical_depth());
    }

    fn use_binding(&mut self, name: &str, span: Span, scopes: &mut Scopes, mode: UseMode) {
        self.use_binding_at_place(name, span, scopes, mode, None);
    }

    fn use_place_base(
        &mut self,
        expr: &Expr,
        scopes: &mut Scopes,
        mode: UseMode,
        places: &[BorrowPlace],
    ) {
        match expr {
            Expr::Variable { name, span } => {
                self.use_binding_at_place(name, *span, scopes, mode, Some(places));
            }
            Expr::This { span } if scopes.get("this").is_some() => {
                self.use_binding_at_place("this", *span, scopes, mode, Some(places));
            }
            Expr::This { span } => {
                for place in places {
                    self.check_live_retained_conflict(place, mode, *span, scopes);
                    self.check_active_borrow_conflict(place, mode, *span);
                    self.check_assignment_write_conflict(place, mode, *span);
                }
            }
            Expr::Grouped { expr, .. } => self.use_place_base(expr, scopes, mode, places),
            Expr::PropertyAccess { object, .. } if self.getter_signature(expr).is_none() => {
                self.use_place_base(object, scopes, mode, places);
            }
            Expr::Index {
                collection, index, ..
            } => {
                self.use_place_base(collection, scopes, mode, places);
                self.use_expr(index, scopes, UseMode::Read);
            }
            _ => {
                self.use_expr_with_pending_access(expr, scopes, mode, false);
                return;
            }
        }
        self.cleanup_expression_result(expr, scopes, None);
    }

    fn use_binding_at_place(
        &mut self,
        name: &str,
        span: Span,
        scopes: &mut Scopes,
        mode: UseMode,
        places: Option<&[BorrowPlace]>,
    ) {
        let roots = places.map(<[BorrowPlace]>::to_vec).unwrap_or_else(|| {
            scopes
                .get(name)
                .map(|binding| binding_roots(binding, name))
                .unwrap_or_default()
        });
        for root in &roots {
            self.check_live_retained_conflict(root, mode, span, scopes);
        }
        {
            let through = scopes
                .get(name)
                .map(|binding| binding.id)
                .into_iter()
                .collect::<Vec<_>>();
            if let Some(alias) = roots
                .iter()
                .find_map(|root| scopes.borrowed_from(root, &through, mode))
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0477",
                        format!(
                            "`${name}` cannot be used as {} while `${alias}` borrows from it",
                            if mode == UseMode::Read { "readonly" } else { "writable" }
                        ),
                        span,
                    )
                    .with_help(
                        "finish using the borrowed binding or place it in a shorter lexical block before mutating its owner",
                    ),
                );
            }
        }
        for root in &roots {
            if matches!(mode, UseMode::Read | UseMode::Write) {
                self.check_active_borrow_conflict(root, mode, span);
            } else if mode == UseMode::Give {
                self.check_give_against_active_borrows(root, span);
            }
        }
        let mut assignment_conflict = false;
        for root in &roots {
            assignment_conflict |= self.check_assignment_write_conflict(root, mode, span);
        }
        if assignment_conflict && mode == UseMode::Give {
            return;
        }
        let nonescaping_value = (mode == UseMode::Give)
            .then(|| {
                scopes
                    .get(name)
                    .and_then(|binding| binding.retained_value.as_ref())
                    .filter(|value| value.nonescaping_parameter)
                    .cloned()
            })
            .flatten();
        if let Some(value) = nonescaping_value {
            let diagnostic = Diagnostic::new(
                "E0657",
                format!("nonescaping callback `${name}` cannot transfer ownership"),
                span,
            )
            .with_title("Nonescaping Callback Cannot Be Retained")
            .with_help("declare the parameter with `take` when ownership must leave the call");
            let diagnostic = self.with_take_parameter_fix(diagnostic, &value);
            self.diagnostics
                .push(self.with_function_value_cause(diagnostic, &value));
            return;
        }
        let Some(binding) = scopes.get_mut(name) else {
            return;
        };
        if mode == UseMode::Write && !binding.writable {
            let diagnostic = if binding.function_type.is_some() {
                Diagnostic::new(
                    "E0656",
                    format!("writable invocation requires writable access to `${name}`"),
                    span,
                )
                .with_title("Writable Invocation Requires Writable Access")
                .with_help("declare the function-value binding writable")
            } else {
                Diagnostic::new(
                    "E0479",
                    format!("readonly `${name}` cannot be used as writable"),
                    span,
                )
                .with_help("declare the binding `writable` before passing it for mutation")
            };
            self.diagnostics.push(diagnostic);
        }
        let maybe_given = matches!(binding.state, State::MaybeGiven { .. });
        match binding.state {
            State::Borrowed if mode == UseMode::Give => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0474",
                        format!("borrowed `${name}` cannot be given away"),
                        span,
                    )
                    .with_help(
                        "declare the parameter with `take` if this function must receive ownership",
                    ),
                );
            }
            State::Borrowed => {}
            State::BorrowedOrOwned if mode == UseMode::Give => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0474",
                        format!("`${name}` may still be borrowed and cannot be given away"),
                        span,
                    )
                    .with_help(
                        "keep borrowed and owned values in separate bindings before transferring ownership",
                    ),
                );
            }
            State::BorrowedOrOwned => {}
            State::Owned if mode == UseMode::Give => {
                binding.state = State::Given { at: span };
            }
            State::Owned => {}
            State::Given { at } | State::MaybeGiven { at } => {
                let diagnostic = if binding.function_type.is_some() {
                    let once = binding.function_type.as_ref().is_some_and(|function| {
                        function.invocation_mode == FunctionInvocationMode::Once
                    });
                    let title = if once && maybe_given {
                        "Once Function May Already Be Consumed"
                    } else if once {
                        "Once Function Was Already Consumed"
                    } else if maybe_given {
                        "Function Value May Already Be Moved"
                    } else {
                        "Function Value Was Already Moved"
                    };
                    let mut diagnostic = Diagnostic::new(
                        "E0655",
                        if maybe_given {
                            format!("function value `${name}` may no longer be available")
                        } else {
                            format!("function value `${name}` is no longer available")
                        },
                        span,
                    )
                    .with_title(title)
                    .with_primary_label("Function Value Used Again Here")
                    .with_related(at, "Function Value Consumed Here")
                    .with_help("move or invoke a function value only once");
                    if let Some(closure_id) = binding
                        .retained_value
                        .as_ref()
                        .and_then(|value| value.closure_id)
                    {
                        diagnostic = diagnostic.with_cause(format!(
                            "closure:{}:{}:ownership",
                            closure_id.start, closure_id.end
                        ));
                    }
                    diagnostic
                } else {
                    Diagnostic::new(
                        "E0470",
                        format!("`${name}` is still being used after its value was given away"),
                        span,
                    )
                    .with_title(format!(
                        "`${name}` Cannot Be Used After Its Value Was Given Away"
                    ))
                    .with_primary_label("Value Used Again Here")
                    .with_explanation(
                        "Giving away a move value ends this binding's ownership, so later reads are invalid.",
                    )
                    .with_related(at, "Value Given Here")
                    .with_help(
                        "A value cannot be used afterward; make its final use happen before ownership is transferred.",
                    )
                };
                self.diagnostics.push(diagnostic);
            }
        }
    }

    fn use_expr_inner(&mut self, expr: &Expr, scopes: &mut Scopes, mode: UseMode) {
        match expr {
            Expr::Variable { name, span } => self.use_binding(name, *span, scopes, mode),
            Expr::This { span } if scopes.get("this").is_some() => {
                self.use_binding("this", *span, scopes, mode);
            }
            Expr::Grouped { expr, .. } => self.use_expr(expr, scopes, mode),
            // Extraction gives away the whole consumed receiver: its other
            // fields are destroyed at once, so no later use of it remains valid.
            Expr::PropertyAccess {
                object,
                take_span: Some(_),
                ..
            } => self.use_expr(object, scopes, UseMode::Give),
            Expr::PropertyAccess {
                object,
                span,
                null_safe,
                ..
            } => {
                if let Some(signature) = self.getter_signature(expr) {
                    let execution = self.method_call_execution(*null_safe, object);
                    self.use_call_args(*span, Some(object), &[], &signature, execution, scopes);
                    self.record_exceptional_exits(*span, scopes);
                    return;
                }
                if mode == UseMode::Write {
                    for place in self.borrow_root_keys(expr, scopes) {
                        self.check_active_borrow_conflict(&place, mode, *span);
                    }
                }
                if mode == UseMode::Give {
                    let source = self.assignment_place_key(expr, scopes);
                    let overlaps_destination = source
                        .as_ref()
                        .is_some_and(|source| self.active_assignment_targets.contains(source));
                    if overlaps_destination {
                        let source = source.expect("overlapping assignment source");
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0471",
                                format!(
                                    "property source `{source}` overlaps assignment destination `{source}`"
                                ),
                                *span,
                            )
                            .with_title("Property Transfer Overlaps Its Destination")
                            .with_help(
                                "move an independently owned value into the property instead",
                            ),
                        );
                    } else if self.expr_is_non_transferable_property(expr, scopes) {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0472",
                                "direct moves out of owned properties are not supported",
                                *span,
                            )
                            .with_help(
                                "use the property without transferring it; moving out requires a separate take-and-replace operation",
                            ),
                        );
                    }
                }
                let assignment_roots = if matches!(mode, UseMode::Read | UseMode::Give)
                    && self
                        .assignment_place_key(expr, scopes)
                        .is_some_and(|target| self.active_assignment_targets.contains(&target))
                {
                    self.borrow_root_keys(expr, scopes)
                } else {
                    Vec::new()
                };
                let suspended_assignment_writes = assignment_roots
                    .into_iter()
                    .filter(|root| self.active_assignment_writes.remove(root))
                    .collect::<Vec<_>>();
                let base_mode = if mode == UseMode::Write {
                    UseMode::Write
                } else {
                    UseMode::Read
                };
                let places = self.borrow_root_keys(expr, scopes);
                if !places.is_empty() {
                    self.use_place_base(object, scopes, base_mode, &places);
                } else {
                    self.use_expr_with_pending_access(object, scopes, base_mode, false);
                }
                self.active_assignment_writes
                    .extend(suspended_assignment_writes);
            }
            Expr::FunctionCall { name, args, span } => {
                let signature = self.signatures.get(name).cloned().unwrap_or_default();
                self.use_call_args(*span, None, args, &signature, CallExecution::Always, scopes);
                self.record_exceptional_exits(*span, scopes);
            }
            Expr::New {
                class_type,
                args,
                span,
                shared,
            } => {
                if *shared {
                    if let Some(value) = self.iterator_value_from_expr(expr, scopes) {
                        self.validate_iterator_storage(&value, *span, "shared ownership");
                    }
                }
                let signature = if class_type.name == "WritableSharedReference" {
                    writable_shared_constructor_signature()
                } else {
                    self.constructors
                        .get(&class_type.name)
                        .cloned()
                        .unwrap_or_default()
                };
                if class_type.name == "WritableSharedReference" {
                    self.use_call_args_with_storage(
                        *span,
                        None,
                        args,
                        &signature,
                        CallExecution::Always,
                        scopes,
                        Some(FunctionStorageBoundary::Deferred {
                            destination: "shared-reference payload storage",
                            title: "Shared Function-Value Payload Is Not Yet Available",
                        }),
                    );
                } else {
                    self.use_call_args(
                        *span,
                        None,
                        args,
                        &signature,
                        CallExecution::Always,
                        scopes,
                    );
                }
                self.record_exceptional_exits(*span, scopes);
            }
            Expr::MethodCall {
                object,
                method,
                member_span,
                args,
                span,
                null_safe,
                ..
            } => {
                if self.callable_value_calls.get(span).is_some_and(|call| {
                    call.target_kind == crate::semantics::CallableValueTargetKind::Property
                }) {
                    let property = Expr::PropertyAccess {
                        object: object.clone(),
                        property: method.clone(),
                        member_span: *member_span,
                        null_safe: false,
                        take_span: None,
                        span: object.span().merge(*member_span),
                    };
                    self.use_callable_value_call(&property, args, *span, scopes);
                    return;
                }
                if self
                    .resolved_type(object)
                    .and_then(|ty| crate::types::ByteRegionKind::method_result(ty, method))
                    .is_some()
                {
                    let (signature, _, _) =
                        self.retained_call(expr, scopes).expect("region signature");
                    let execution = self.method_call_execution(*null_safe, object);
                    self.use_call_args(*span, Some(object), args, &signature, execution, scopes);
                    return;
                }
                if let Some(collection) = self.expr_collection_info(object, scopes) {
                    self.use_collection_call(
                        collection.family,
                        object,
                        method,
                        args,
                        *span,
                        self.method_call_execution(*null_safe, object),
                        scopes,
                    );
                    return;
                }
                let signature = self
                    .retained_call(expr, scopes)
                    .map(|(signature, _, _)| signature)
                    .unwrap_or_default();
                let execution = self.method_call_execution(*null_safe, object);
                self.use_call_args(*span, Some(object), args, &signature, execution, scopes);
                self.record_exceptional_exits(*span, scopes);
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                span,
                ..
            } => {
                if matches!(
                    qualifier,
                    ast::StaticQualifier::Class(class_name)
                        if (class_name == "Set" && method == "from")
                            || (class_name == "Bytes"
                                && matches!(method.as_str(), "fromArray" | "allocate"))
                ) {
                    for arg in args {
                        self.use_expr(&arg.value, scopes, UseMode::Read);
                    }
                    self.record_exceptional_exits(*span, scopes);
                    return;
                }
                let signature = self
                    .retained_call(expr, scopes)
                    .map(|(signature, _, _)| signature)
                    .or_else(|| self.static_call_signature(*span, qualifier, method))
                    .unwrap_or_default();
                if method == "__construct" && matches!(qualifier, ast::StaticQualifier::Parent) {
                    self.retain_parent_constructor_sources(*span, &signature, args, scopes);
                }
                let enum_payload = self
                    .qualifier_class(qualifier)
                    .is_some_and(|class| self.enum_cases.contains_key(&(class, method.clone())));
                if enum_payload {
                    self.use_call_args_with_storage(
                        *span,
                        None,
                        args,
                        &signature,
                        CallExecution::Always,
                        scopes,
                        Some(FunctionStorageBoundary::Transport("an enum payload")),
                    );
                    if let Some(case) = self.enum_case_constructions.get(span).copied() {
                        let bound = signature.bind_arguments(args);
                        for (index, argument) in args.iter().enumerate() {
                            if let Some(field) = bound.arg_to_param.get(index).copied().flatten() {
                                self.cleanup_analysis.source_flows.insert((
                                    cleanup::Source::Expression(argument.value.span()),
                                    cleanup::Source::EnumPayload {
                                        scrutinee: *span,
                                        case,
                                        field,
                                    },
                                ));
                            }
                        }
                    }
                } else {
                    self.use_call_args(
                        *span,
                        None,
                        args,
                        &signature,
                        CallExecution::Always,
                        scopes,
                    );
                }
                self.record_exceptional_exits(*span, scopes);
            }
            Expr::InterpolatedString { parts, .. } => {
                let borrow_depth = self.active_borrows.len();
                for part in parts {
                    if let ast::InterpolatedStringPart::Expr(expr) = part {
                        self.use_read_with_place_borrow(expr, scopes);
                    }
                }
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::Array { elements, .. } => {
                let borrow_depth = self.active_borrows.len();
                for element in elements {
                    if let Some(key) = &element.key {
                        let mode = self.use_owned_expression(key, scopes);
                        self.activate_place_input_borrows(key, scopes);
                        if mode == UseMode::Read {
                            self.activate_borrow(key, mode, scopes);
                        }
                    }
                    let mode = self.use_owned_expression(&element.value, scopes);
                    if let Some(value) = self.retained_value_from_expr(&element.value, scopes) {
                        self.validate_collection_element(
                            &value,
                            element.value.span(),
                            scopes.lexical_depth(),
                        );
                    }
                    self.activate_place_input_borrows(&element.value, scopes);
                    if mode == UseMode::Read {
                        self.activate_borrow(&element.value, mode, scopes);
                    }
                }
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::ArrayRepeat { value, count, .. } => {
                let borrow_depth = self.active_borrows.len();
                let retained = self.evaluate_retained_value(value, scopes, UseMode::Read);
                if let Some(function) = retained {
                    self.validate_collection_element(
                        &function,
                        value.span(),
                        scopes.lexical_depth(),
                    );
                }
                self.activate_place_input_borrows(value, scopes);
                self.activate_borrow(value, UseMode::Read, scopes);
                self.use_read_with_place_borrow(count, scopes);
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::Index {
                collection, index, ..
            } => {
                self.use_expr(
                    collection,
                    scopes,
                    if mode == UseMode::Write {
                        UseMode::Write
                    } else {
                        UseMode::Read
                    },
                );
                self.use_expr(index, scopes, UseMode::Read);
            }
            Expr::Unary { expr, .. } => self.use_expr(expr, scopes, UseMode::Read),
            Expr::IsType { expr, .. } => self.use_expr(expr, scopes, UseMode::Read),
            Expr::Binary {
                left,
                op: op @ (BinaryOp::And | BinaryOp::Or),
                right,
                ..
            } => {
                let borrow_depth = self.active_borrows.len();
                self.use_expr(left, scopes, UseMode::Read);
                // The left boolean is consumed before the conditional right
                // operand runs; enclosing call-argument borrows stay live.
                self.active_borrows.truncate(borrow_depth);
                match (op, constant_bool(left)) {
                    (BinaryOp::And, Some(false)) | (BinaryOp::Or, Some(true)) => {}
                    (BinaryOp::And, Some(true)) | (BinaryOp::Or, Some(false)) => {
                        self.use_expr(right, scopes, UseMode::Read);
                    }
                    _ => {
                        let without_right = scopes.clone();
                        let mut with_right = without_right.clone();
                        self.use_expr(right, &mut with_right, UseMode::Read);
                        scopes.merge_from(&without_right, &with_right);
                    }
                }
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => {
                match self.flow_fact(left) {
                    Some(Fact::NonNull | Fact::Exact(_) | Fact::Constructed { .. }) => {
                        self.use_expr(left, scopes, mode);
                        return;
                    }
                    Some(Fact::Null) => {
                        self.use_expr(left, scopes, UseMode::Read);
                        self.use_expr(right, scopes, mode);
                        return;
                    }
                    None if matches!(ungroup_expr(left), Expr::Null { .. }) => {
                        self.use_expr(left, scopes, UseMode::Read);
                        self.use_expr(right, scopes, mode);
                        return;
                    }
                    None => {}
                }
                let before = scopes.clone();
                let mut selected = before.clone();
                self.use_expr(left, &mut selected, mode);
                let mut fallback = before;
                self.use_expr(left, &mut fallback, UseMode::Read);
                self.use_expr(right, &mut fallback, mode);
                scopes.merge_from(&selected, &fallback);
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
                ..
            } => {
                let borrow_depth = self.active_borrows.len();
                self.use_read_with_place_borrow(left, scopes);
                self.use_expr(right, scopes, UseMode::Read);
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::This { span } => {
                self.check_live_retained_conflict(&"$this".into(), mode, *span, scopes);
                if matches!(mode, UseMode::Read | UseMode::Write) {
                    self.check_active_borrow_conflict(&"$this".into(), mode, *span);
                } else if mode == UseMode::Give {
                    self.check_give_against_active_borrows(&"$this".into(), *span);
                }
                if self.check_assignment_write_conflict(&"$this".into(), mode, *span)
                    && mode == UseMode::Give
                {
                    return;
                }
                if mode == UseMode::Give {
                    self.diagnostics.push(
                        Diagnostic::new("E0474", "borrowed `$this` cannot be given away", *span)
                            .with_help(
                                "the method receiver is borrowed from its caller and must remain owned by that caller",
                            ),
                    );
                }
            }
            Expr::StaticMember { span, .. } => {
                let Some(root) = self.borrow_root_key(expr, scopes) else {
                    return;
                };
                let exact_assignment_target = mode == UseMode::Read
                    && self
                        .assignment_place_key(expr, scopes)
                        .is_some_and(|target| self.active_assignment_targets.contains(&target));
                let suspended_assignment_write =
                    exact_assignment_target && self.active_assignment_writes.remove(&root);
                if matches!(mode, UseMode::Read | UseMode::Write) {
                    self.check_active_borrow_conflict(&root, mode, *span);
                } else {
                    self.check_give_against_active_borrows(&root, *span);
                }
                self.check_assignment_write_conflict(&root, mode, *span);
                if suspended_assignment_write {
                    self.active_assignment_writes.insert(root);
                }
            }
            Expr::Closure(closure) => {
                let closure_id = ClosureId::from_span(closure.span);
                if !self.prepared_closure_evaluations.remove(&closure_id) {
                    self.acquire_closure(closure, scopes);
                }
                if self.closure_values.contains_key(&closure_id) {
                    self.analyze_closure_body(closure, scopes);
                }
            }
            Expr::CallableCall {
                callee, args, span, ..
            } => {
                if !self.use_callable_value_call(callee, args, *span, scopes) {
                    self.use_expr(callee, scopes, UseMode::Read);
                    for argument in args {
                        self.use_expr(&argument.value, scopes, UseMode::Read);
                    }
                }
            }
            Expr::Match {
                scrutinee,
                mode: match_mode,
                arms,
                span,
                ..
            } => {
                let value =
                    self.use_match_expression(*span, scrutinee, *match_mode, arms, scopes, mode);
                self.retained.expression_values.remove(span);
                if let Some(value) = value {
                    self.retained.expression_values.insert(*span, value);
                }
            }
            Expr::When(when) => {
                let result_depth = scopes.lexical_depth();
                self.cleanup_yield_regions.push((when.span, result_depth));
                let has_given = when.given.is_some();
                if let Some(given) = &when.given {
                    scopes.push();
                    self.check_given_setup(given, scopes, false);
                }
                let before = scopes.clone();
                let mut outcomes = Vec::new();
                self.when_result_modes.push(mode);
                self.retained.yielded_values.push(None);
                for branch in &when.branches {
                    let mut branch_scopes = before.clone();
                    if let Some(condition) = &branch.condition {
                        self.use_expr(condition, &mut branch_scopes, UseMode::Read);
                    }
                    let mut branch_flow =
                        self.check_block(&branch.block, &mut branch_scopes, false, true);
                    if let Some(finally) = &when.finally {
                        branch_flow = self.apply_finally_to_flow(
                            &finally.block,
                            &mut branch_scopes,
                            branch_flow,
                            false,
                        );
                    }
                    outcomes.extend(branch_flow.yields);
                }
                self.when_result_modes.pop();
                self.retained.expression_values.remove(&when.span);
                if let Some(value) = self.retained.yielded_values.pop().flatten() {
                    self.retained.expression_values.insert(when.span, value);
                }
                if let Some(first) = outcomes.first().cloned() {
                    let mut merged = first;
                    for outcome in outcomes.iter().skip(1) {
                        let current = merged.clone();
                        merged.merge_from(&current, outcome);
                    }
                    *scopes = merged;
                }
                if has_given {
                    self.cleanup_pop_scope(scopes, when.span, cleanup::Cause::ScopeExit);
                }
                self.cleanup_yield_regions.pop();
                let borrows = scopes
                    .1
                    .take_borrows(&cleanup::Slot::PendingYield(when.span));
                let result = scopes.1.take(&cleanup::Slot::PendingYield(when.span));
                scopes
                    .1
                    .put(cleanup::Slot::Temporary(when.span), result, result_depth);
                scopes
                    .1
                    .put_borrows(cleanup::Slot::Temporary(when.span), borrows, result_depth);
            }
            Expr::Identifier { .. }
            | Expr::String { .. }
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => {}
        }
    }

    fn use_callable_value_call(
        &mut self,
        callee: &Expr,
        args: &[Argument],
        span: Span,
        scopes: &mut Scopes,
    ) -> bool {
        let Some(call) = self.callable_value_calls.get(&span).cloned() else {
            return false;
        };
        let function_type = non_null_function_type(&call.function_type).cloned();
        let pending_closure = self
            .prepare_retained_value(callee, scopes)
            .and_then(|value| value.closure_id);
        let callee_mode = match call.invocation_mode {
            FunctionInvocationMode::Readonly => UseMode::Read,
            FunctionInvocationMode::Writable => UseMode::Write,
            FunctionInvocationMode::Once => UseMode::Give,
        };
        let stored_once_source = call.invocation_mode == FunctionInvocationMode::Once
            && self.getter_signature(callee).is_none()
            && matches!(
                ungroup_expr(callee),
                Expr::PropertyAccess { .. } | Expr::Index { .. }
            );
        let mut borrowed_once_source = false;
        if call.invocation_mode == FunctionInvocationMode::Once {
            if let Expr::Variable { name, .. } = ungroup_expr(callee) {
                if scopes.get(name).is_some_and(|binding| {
                    binding.borrowed_place
                        || binding
                            .retained_value
                            .as_ref()
                            .is_some_and(|value| value.nonescaping_parameter)
                }) {
                    borrowed_once_source = true;
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0656",
                            format!("once invocation of `${name}` requires ownership"),
                            callee.span(),
                        )
                        .with_title("Once Invocation Requires Ownership")
                        .with_help(
                            "accept the callback through a `take function once(...)` parameter",
                        ),
                    );
                }
            }
        }
        self.use_expr(
            callee,
            scopes,
            if stored_once_source || borrowed_once_source {
                UseMode::Read
            } else {
                callee_mode
            },
        );
        if callee_mode == UseMode::Give && !stored_once_source && !borrowed_once_source {
            let values = self.cleanup_take_expr(callee, scopes);
            scopes.1.put(
                cleanup::Slot::PendingCallee(span),
                values,
                scopes.lexical_depth(),
            );
        }
        if let Some(function_type) = function_type {
            self.use_callable_args(span, args, &function_type, scopes);
            self.apply_callable_writeback(span, callee, args, &function_type, scopes);
            let mut pending_targets = self
                .callback_capture_targets
                .get(&span)
                .cloned()
                .unwrap_or_default();
            if pending_targets.is_empty() {
                if let Some(closure) = pending_closure {
                    pending_targets.push(CallbackCaptureTarget {
                        declaration: Span {
                            source: closure.source,
                            expansion: closure.expansion,
                            start: closure.start,
                            end: closure.end,
                        },
                        captures: Vec::new(),
                    });
                }
            }
            for target in pending_targets {
                let signature = Signature::for_function_value(
                    &function_type,
                    Some(target.declaration),
                    &self.move_enum_names,
                );
                self.retain_pending_call_inputs(
                    span,
                    None,
                    args,
                    &signature,
                    Some((callee, &target)),
                    scopes,
                );
            }
            if self.retained.implementations.contains_key(&span) {
                let signature = Signature::for_function_value(
                    &function_type,
                    Some(span),
                    &self.move_enum_names,
                );
                self.transfer_stored_call_sources(span, None, args, &signature, scopes);
            }
        } else {
            for argument in args {
                self.use_expr(&argument.value, scopes, UseMode::Read);
            }
        }
        // The environment stays caller-owned while arguments are evaluated.
        // Only invocation transfers it to the once body, including its unwind.
        scopes.1.take(&cleanup::Slot::PendingCallee(span));
        self.record_exceptional_exits(span, scopes);
        true
    }

    fn use_match_expression(
        &mut self,
        site: Span,
        scrutinee: &Expr,
        match_mode: ast::MatchMode,
        arms: &[ast::MatchArm],
        scopes: &mut Scopes,
        mode: UseMode,
    ) -> Option<RetainedValueState> {
        let borrow_depth = self.active_borrows.len();
        let consuming = matches!(match_mode, ast::MatchMode::Consumed { .. });
        let checked = self.matches.get(&site).cloned();
        let enum_definition = checked.as_ref().and_then(|info| {
            let ty = match &info.scrutinee_type {
                ResolvedType::Nullable(inner) => inner.as_ref(),
                ty => ty,
            };
            let ResolvedType::Enum(ty) = ty else {
                return None;
            };
            self.checked_enum_application(ty).cloned()
        });
        let mut remaining_cases = enum_definition
            .as_ref()
            .map(|definition| {
                definition
                    .cases
                    .iter()
                    .map(|case| crate::enums::EnumCaseSource {
                        declaration: definition.source_type.id,
                        index: case.id.index,
                    })
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        self.use_expr(
            scrutinee,
            scopes,
            if consuming {
                UseMode::Give
            } else {
                UseMode::Read
            },
        );
        let retained_scrutinee = self.retained_value_from_expr(scrutinee, scopes);
        let borrow_root = self.borrow_source_root(scrutinee, scopes);
        if !consuming {
            self.activate_place_borrow(scrutinee, UseMode::Read, scopes);
        }

        let mut remaining = scopes.clone();
        let mut outcomes = Vec::with_capacity(arms.len());
        let mut has_default = false;
        let mut result_value = None;
        for (arm_index, arm) in arms.iter().enumerate() {
            let arm_info = checked.as_ref().and_then(|info| info.arms.get(arm_index));
            if let ast::MatchPattern::Expression(pattern) = &arm.pattern {
                self.use_expr(pattern, &mut remaining, UseMode::Read);
            }

            let mut selected = remaining.clone();
            let scrutinee_slot = cleanup::Slot::Temporary(scrutinee.span());
            let original_scrutinee = selected.1.values(&scrutinee_slot);
            let mut payloads = HashMap::new();
            if arm_info.is_some_and(|info| {
                matches!(info.pattern, crate::semantics::ResolvedMatchPattern::Null)
            }) {
                // The selected null path carries no owned payload. Preserve
                // the original on the unmatched/failed-guard path only.
                selected.1.take(&scrutinee_slot);
            }
            if !original_scrutinee.is_empty() {
                if let (Some(owner), Some(definition), Some(arm_info)) = (
                    self.cleanup_owner,
                    enum_definition.as_ref(),
                    arm_info.filter(|info| {
                        matches!(
                            info.pattern,
                            crate::semantics::ResolvedMatchPattern::EnumCase { .. }
                                | crate::semantics::ResolvedMatchPattern::Default
                                | crate::semantics::ResolvedMatchPattern::Null
                        )
                    }),
                ) {
                    let cases = match &arm_info.pattern {
                        crate::semantics::ResolvedMatchPattern::EnumCase { case_id, .. } => {
                            HashSet::from([*case_id])
                        }
                        crate::semantics::ResolvedMatchPattern::Default => remaining_cases.clone(),
                        crate::semantics::ResolvedMatchPattern::Null => HashSet::new(),
                        _ => remaining_cases.clone(),
                    };
                    let mut residual = HashSet::new();
                    for case in definition.cases.iter().filter(|case| {
                        cases.contains(&crate::enums::EnumCaseSource {
                            declaration: definition.source_type.id,
                            index: case.id.index,
                        })
                    }) {
                        let source_case = crate::enums::EnumCaseSource {
                            declaration: definition.source_type.id,
                            index: case.id.index,
                        };
                        for (field, payload) in case.payload.iter().enumerate() {
                            if !self.checked_payload_requires_move(&payload.ty) {
                                continue;
                            }
                            let value = self.cleanup_analysis.acquire(
                                owner,
                                cleanup::Source::EnumPayload {
                                    scrutinee: scrutinee.span(),
                                    case: source_case,
                                    field,
                                },
                                payload.ty.clone(),
                                HashSet::new(),
                            );
                            residual.insert(value);
                            payloads.insert((source_case, field), value);
                        }
                    }
                    selected
                        .1
                        .put(scrutinee_slot, residual, scopes.lexical_depth());
                }
            }
            if let Some(guard) = &arm.guard {
                selected.push();
                for (field, binding) in match_pattern_bindings(&arm.pattern).into_iter().enumerate()
                {
                    let source = arm_info.and_then(|info| match info.pattern {
                        crate::semantics::ResolvedMatchPattern::EnumCase { case_id, .. } => {
                            Some(cleanup::Source::EnumPayload {
                                scrutinee: scrutinee.span(),
                                case: case_id,
                                field,
                            })
                        }
                        crate::semantics::ResolvedMatchPattern::ExactType(_) => {
                            Some(cleanup::Source::Expression(scrutinee.span()))
                        }
                        _ => None,
                    });
                    self.declare_match_binding(
                        binding,
                        borrow_root.clone(),
                        true,
                        source,
                        None,
                        retained_scrutinee.as_ref(),
                        &mut selected,
                    );
                }
                self.use_expr(&guard.condition, &mut selected, UseMode::Read);
                self.cleanup_pop_scope(
                    &mut selected,
                    guard.condition.span(),
                    cleanup::Cause::ScopeExit,
                );

                let before_guard = remaining.clone();
                let mut failed_guard = selected.clone();
                // A guard is a view: failure leaves the whole scrutinee owned
                // by the selection region for the following arm.
                failed_guard
                    .1
                    .put(scrutinee_slot, original_scrutinee, scopes.lexical_depth());
                remaining.merge_from(&before_guard, &failed_guard);
                if arm_info.is_some_and(|info| {
                    info.guard == crate::semantics::MatchGuardSemanticInfo::AlwaysFalse
                }) {
                    continue;
                }
            }

            selected.push();
            for (field, binding) in match_pattern_bindings(&arm.pattern).into_iter().enumerate() {
                let binding_info = arm_info.and_then(|info| info.bindings.get(field));
                let borrowed = binding_info.map_or(!consuming, |info| info.borrowed);
                let source = arm_info.and_then(|info| match info.pattern {
                    crate::semantics::ResolvedMatchPattern::EnumCase { case_id, .. } => {
                        Some(cleanup::Source::EnumPayload {
                            scrutinee: scrutinee.span(),
                            case: case_id,
                            field,
                        })
                    }
                    crate::semantics::ResolvedMatchPattern::ExactType(_) => {
                        Some(cleanup::Source::Expression(scrutinee.span()))
                    }
                    _ => None,
                });
                let mut transferred = None;
                if consuming && !borrowed {
                    match arm_info.map(|info| &info.pattern) {
                        Some(crate::semantics::ResolvedMatchPattern::EnumCase {
                            case_id, ..
                        }) => {
                            let value = payloads.get(&(*case_id, field)).copied();
                            let mut residual = selected.1.take(&scrutinee_slot);
                            if let Some(value) = value {
                                residual.remove(&value);
                            }
                            selected
                                .1
                                .put(scrutinee_slot, residual, scopes.lexical_depth());
                            transferred = Some(value.into_iter().collect());
                        }
                        Some(crate::semantics::ResolvedMatchPattern::ExactType(ty))
                            if resolved_type_is_move_type(ty, &self.move_enum_names)
                                || resolved_type_requires_conservative_move(ty) =>
                        {
                            let values = selected.1.take(&scrutinee_slot);
                            if values.iter().all(|value| {
                                self.cleanup_analysis
                                    .values
                                    .get(value)
                                    .is_some_and(|value| value.ty == *ty)
                            }) {
                                transferred = Some(values);
                            } else if let (Some(owner), Some(binding)) =
                                (self.cleanup_owner, self.canonical_binding_id(binding.span))
                            {
                                let contents = values
                                    .iter()
                                    .filter_map(|value| self.cleanup_analysis.values.get(value))
                                    .flat_map(|value| value.contents.iter().copied())
                                    .collect();
                                let value = self.cleanup_analysis.acquire(
                                    owner,
                                    cleanup::Source::Binding(binding),
                                    ty.clone(),
                                    contents,
                                );
                                transferred = Some(HashSet::from([value]));
                            }
                        }
                        _ => {}
                    }
                }
                self.declare_match_binding(
                    binding,
                    borrow_root.clone(),
                    borrowed,
                    source,
                    transferred,
                    retained_scrutinee.as_ref(),
                    &mut selected,
                );
            }
            let result_mode = if consuming
                && checked.as_ref().is_some_and(|info| {
                    resolved_type_is_move_type(&info.result_type, &self.move_enum_names)
                        || resolved_type_requires_conservative_move(&info.result_type)
                }) {
                UseMode::Give
            } else {
                mode
            };
            let value = self.evaluate_retained_value(&arm.value, &mut selected, result_mode);
            result_value = join_retained_value(result_value.as_ref(), value.as_ref());
            let result = self.cleanup_take_expr(&arm.value, &mut selected);
            for value in &result {
                if let Some(value) = self.cleanup_analysis.values.get(value) {
                    self.cleanup_analysis
                        .source_flows
                        .insert((value.source, cleanup::Source::Expression(site)));
                }
            }
            selected.1.put(
                cleanup::Slot::Temporary(site),
                result,
                scopes.lexical_depth(),
            );
            self.cleanup_pop_scope(&mut selected, arm.value.span(), cleanup::Cause::ScopeExit);
            outcomes.push(selected);

            let covers_pattern = arm_info.is_none_or(|info| {
                matches!(
                    info.guard,
                    crate::semantics::MatchGuardSemanticInfo::None
                        | crate::semantics::MatchGuardSemanticInfo::AlwaysTrue
                )
            });
            if covers_pattern {
                if let Some(crate::semantics::ResolvedMatchPattern::EnumCase { case_id, .. }) =
                    arm_info.map(|info| &info.pattern)
                {
                    remaining_cases.remove(case_id);
                }
                if let Some(crate::semantics::ResolvedMatchPattern::ExactType(ty)) =
                    arm_info.map(|info| &info.pattern)
                {
                    let mut values = remaining.1.take(&scrutinee_slot);
                    values.retain(|value| {
                        self.cleanup_analysis.values.get(value).is_none_or(|value| {
                            // Removing T from ?T leaves only null, which
                            // cannot run T's destructor in a later arm.
                            // An open mixed/interface remainder is still
                            // retained; matching one type cannot clear it.
                            value.ty != *ty
                                && !matches!(&value.ty, ResolvedType::Nullable(inner)
                                        if inner.as_ref() == ty)
                        })
                    });
                    remaining
                        .1
                        .put(scrutinee_slot, values, scopes.lexical_depth());
                }
            }
            if covers_pattern && matches!(arm.pattern, ast::MatchPattern::Default { .. }) {
                has_default = true;
                break;
            }
        }

        // Checked matches are exhaustive; MIR's final fallback is unreachable.
        // An invented unmatched path would drop values every real arm moved.
        if !has_default && checked.is_none() {
            outcomes.push(remaining);
        }
        if let Some(mut joined) = outcomes.pop() {
            for outcome in outcomes {
                let previous = joined.clone();
                joined.merge_from(&previous, &outcome);
            }
            *scopes = joined;
        }
        self.active_borrows.truncate(borrow_depth);
        result_value
    }

    #[allow(clippy::too_many_arguments)]
    fn declare_match_binding(
        &mut self,
        binding: &ast::MatchBinding,
        borrow_root: Option<BorrowSourceRoot>,
        borrowed: bool,
        source: Option<cleanup::Source>,
        transferred: Option<HashSet<cleanup::ValueId>>,
        retained_scrutinee: Option<&RetainedValueState>,
        scopes: &mut Scopes,
    ) {
        let Some(ty) = self.resolved_types.get(&binding.span) else {
            return;
        };
        let canonical_id = self.canonical_binding_id(binding.span);
        let retained_payload = match source {
            Some(cleanup::Source::EnumPayload { case, field, .. }) => {
                retained_scrutinee.and_then(|value| {
                    let kind = if non_null_function_type(ty).is_some() {
                        Some(RetainedValueKind::Closure)
                    } else if self.retained.source_lifetime(ty).is_some() {
                        Some(RetainedValueKind::SourceCarrier)
                    } else {
                        None
                    };
                    value.project_enum_payload(case, field, kind)
                })
            }
            _ => None,
        }
        .map(|mut value| {
            // The projection carries the payload callback's own loans. A
            // borrowed binding still only views the scrutinee's callback.
            value.nonescaping_parameter |= borrowed && non_null_function_type(ty).is_some();
            value
        });
        scopes.declare(
            binding.name.clone(),
            Binding {
                id: self.next_binding_id(),
                canonical_id,
                class: resolved_type_class(ty).map(str::to_string),
                collection: resolved_collection_info(ty, &self.move_enum_names),
                mixed: resolved_type_is_mixed(ty),
                borrowed_place: borrowed,
                borrow_root: borrowed.then_some(borrow_root).flatten(),
                borrow_parents: Vec::new(),
                writeback_homes: Vec::new(),
                writable: false,
                state: if borrowed {
                    State::Borrowed
                } else {
                    State::Owned
                },
                function_type: non_null_function_type(ty).cloned(),
                retained_value: retained_payload.or_else(|| {
                    self.parameter_function_value(
                        canonical_id,
                        if borrowed {
                            BindingOwnership::ReadonlyBorrow
                        } else {
                            BindingOwnership::Owned
                        },
                        None,
                    )
                }),
                scope_depth: scopes.lexical_depth(),
            },
        );
        if let Some(binding) = canonical_id {
            let destination = cleanup::Source::Binding(binding);
            // A match binding denotes its selected payload, including while a
            // guard only lends it. Whole-enum flow would merge sibling fields'
            // callback origins and charge unrelated cleanup to this binding.
            if let Some(source) = source {
                self.cleanup_analysis
                    .source_flows
                    .insert((source, destination));
            }
            for value in transferred.iter().flatten() {
                if let Some(value) = self.cleanup_analysis.values.get(value) {
                    self.cleanup_analysis
                        .source_flows
                        .insert((value.source, destination));
                }
            }
        }
        if !borrowed {
            if let Some(binding) = canonical_id {
                if let Some(values) = transferred {
                    scopes.1.put(
                        cleanup::Slot::Binding(binding),
                        values,
                        scopes.lexical_depth(),
                    );
                } else {
                    self.cleanup_owned_parameter(binding, scopes);
                }
            }
        }
    }

    fn analyze_closure_body(
        &mut self,
        closure: &ast::ClosureExpression,
        enclosing_scopes: &Scopes,
    ) {
        let closure_id = ClosureId::from_span(closure.span);
        if !self.analyzed_closures.insert(closure_id) {
            return;
        }
        let Some(semantic) = self.closures.get(&closure_id).cloned() else {
            return;
        };
        let cleanup_context = self.cleanup_begin_callable(closure.span);
        let previous_retained_context = self.retained.enter_closure(
            closure,
            self.binding_resolution,
            non_null_function_type(&semantic.function_type)
                .and_then(|function| function.return_borrow),
        );
        let mut scopes = Scopes::new();
        for capture in &semantic.captures {
            let Some(declaration) = self
                .binding_resolution
                .declarations_by_id
                .get(&capture.environment_binding_id)
            else {
                continue;
            };
            if declaration.kind == BindingKind::MethodReceiver
                && capture.mode != ast::ClosureCaptureMode::Take
            {
                continue;
            }
            let source_value = enclosing_scopes
                .get_by_canonical(capture.source_binding_id)
                .and_then(|(_, binding)| binding.retained_value.clone());
            let borrowed = capture.mode != ast::ClosureCaptureMode::Take;
            let retained_value = source_value
                .or_else(|| {
                    owned_storage::is_selector_type(&capture.source_type)
                        .then(|| retained::iterator_value(Vec::new()))
                })
                .map(|mut value| {
                    // A capture is not an argument of the invoked closure. Keep
                    // its storage loan, but do not attribute returns to an outer
                    // function parameter that happens to have the same name.
                    for loan in &mut value.leases {
                        loan.root = BorrowRoot::EnclosingEnvironment(closure_id);
                        loan.pending_capture = Some(capture.environment_binding_id);
                    }
                    if borrowed {
                        value.provenance =
                            ValueProvenance::BorrowBound(vec![BorrowRoot::EnclosingEnvironment(
                                closure_id,
                            )]);
                        value.nonescaping_parameter = false;
                    }
                    if owned_storage::is_selector_type(&capture.source_type) {
                        value
                            .storage
                            .capture_selector(capture.environment_binding_id);
                    }
                    value
                });
            scopes.declare(
                declaration.name.clone(),
                Binding {
                    id: self.next_binding_id(),
                    canonical_id: Some(capture.environment_binding_id),
                    class: resolved_type_class(&capture.source_type).map(str::to_string),
                    collection: resolved_collection_info(
                        &capture.source_type,
                        &self.move_enum_names,
                    ),
                    mixed: resolved_type_is_mixed(&capture.source_type),
                    borrowed_place: borrowed,
                    borrow_root: borrowed.then(|| {
                        BorrowSourceRoot::Place(
                            format!(
                                "closure:{}:{}:binding:{}",
                                closure_id.start, closure_id.end, capture.environment_binding_id.0
                            )
                            .into(),
                        )
                    }),
                    borrow_parents: Vec::new(),
                    writeback_homes: Vec::new(),
                    writable: capture.mode != ast::ClosureCaptureMode::Readonly,
                    state: if borrowed {
                        State::Borrowed
                    } else {
                        State::Owned
                    },
                    function_type: non_null_function_type(&capture.source_type).cloned(),
                    retained_value,
                    scope_depth: scopes.lexical_depth(),
                },
            );
            if !borrowed {
                self.cleanup_owned_parameter(capture.environment_binding_id, &mut scopes);
                if semantic.inferred_invocation_mode != FunctionInvocationMode::Once {
                    // Repeatable captures belong to the surviving environment,
                    // not to an individual invocation's lexical frame.
                    let slot = cleanup::Slot::Binding(capture.environment_binding_id);
                    let values = scopes.1.take(&slot);
                    scopes.1.put(slot, values, 0);
                }
            }
        }
        for parameter in &closure.parameters {
            let canonical_id = self.canonical_binding_id(parameter.name_span);
            let Some(declaration) =
                canonical_id.and_then(|id| self.binding_resolution.declarations_by_id.get(&id))
            else {
                continue;
            };
            let Some(source_type) = declaration.source_type.as_ref() else {
                continue;
            };
            let ownership = if parameter.take {
                BindingOwnership::Owned
            } else if parameter.writable {
                BindingOwnership::WritableBorrow
            } else {
                BindingOwnership::ReadonlyBorrow
            };
            scopes.declare(
                parameter.name.clone(),
                Binding {
                    id: self.next_binding_id(),
                    canonical_id,
                    class: resolved_type_class(source_type).map(str::to_string),
                    collection: resolved_collection_info(source_type, &self.move_enum_names),
                    mixed: resolved_type_is_mixed(source_type),
                    borrowed_place: ownership != BindingOwnership::Owned,
                    borrow_root: None,
                    borrow_parents: Vec::new(),
                    writeback_homes: Vec::new(),
                    writable: parameter.writable,
                    state: if ownership == BindingOwnership::Owned {
                        State::Owned
                    } else {
                        State::Borrowed
                    },
                    function_type: non_null_function_type(source_type).cloned(),
                    retained_value: self
                        .parameter_function_value(
                            canonical_id,
                            ownership,
                            (!parameter.take && !parameter.writable).then_some(Span::new(
                                parameter.type_span.start,
                                parameter.type_span.start,
                            )),
                        )
                        .or_else(|| self.parameter_retained_value(canonical_id)),
                    scope_depth: scopes.lexical_depth(),
                },
            );
            if parameter.take {
                if let Some(binding) = canonical_id {
                    self.cleanup_owned_parameter(binding, &mut scopes);
                }
            }
        }

        self.prepare_conditional_borrow_sources(
            &closure_borrow_function(closure, self.current_type_params.clone()),
            &scopes,
        );
        let previous_return_borrow = self.current_return_borrow;
        self.current_return_borrow = non_null_function_type(&semantic.function_type)
            .and_then(|function| function.return_borrow)
            .filter(|borrow| borrow.kind == ReturnBorrowKind::Value)
            .map(|borrow| {
                if borrow.writable {
                    UseMode::Write
                } else {
                    UseMode::Read
                }
            });
        let return_move_type =
            resolved_type_is_move_type(&semantic.inferred_return_type, &self.move_enum_names)
                && self.current_return_borrow.is_none();
        let mut flow = match &closure.body {
            ast::ClosureBody::Expression { expression, .. } => {
                if let Some(value) = self.prepare_retained_value(expression, &mut scopes) {
                    let _ = self.validate_returned_function_value(&value, expression.span());
                }
                let temporary_borrow = self.reject_returned_temporary_borrow(expression, &scopes);
                self.use_expr(
                    expression,
                    &mut scopes,
                    if temporary_borrow {
                        UseMode::Read
                    } else if return_move_type {
                        UseMode::Give
                    } else {
                        self.current_return_borrow.unwrap_or(UseMode::Read)
                    },
                );
                self.record_writeback_return(expression, &scopes);
                if let Some(value) = self.retained_value_from_expr(expression, &scopes) {
                    self.record_returned_storage_selectors(&value);
                }
                self.cleanup_stage_return(Some(expression), &mut scopes, expression.span());
                Flow::returns(&scopes)
            }
            ast::ClosureBody::Block(block) => {
                self.prepare_retained_liveness(block, closure.span);
                self.check_block(block, &mut scopes, return_move_type, false)
            }
        };
        self.summarize_stored_call_sources(&scopes, &flow);
        self.cleanup_end_callable(cleanup_context, &mut scopes, &mut flow, closure.span);
        self.current_return_borrow = previous_return_borrow;
        self.retained.leave_callable(previous_retained_context);
    }

    fn use_callable_args(
        &mut self,
        site: Span,
        args: &[Argument],
        function: &crate::types::SemanticFunctionType<ResolvedType>,
        scopes: &mut Scopes,
    ) {
        let borrow_depth = self.active_borrows.len();
        for (index, argument) in args.iter().enumerate() {
            let mode = function
                .parameters
                .get(index)
                .map(|parameter| match parameter.ownership_mode {
                    crate::types::FunctionTypeParameterMode::Readonly => UseMode::Read,
                    crate::types::FunctionTypeParameterMode::Writable => UseMode::Write,
                    crate::types::FunctionTypeParameterMode::Take => {
                        if resolved_type_is_move_type(&parameter.ty, &self.move_enum_names)
                            || resolved_type_requires_conservative_move(&parameter.ty)
                        {
                            UseMode::Give
                        } else {
                            UseMode::Read
                        }
                    }
                })
                .unwrap_or(UseMode::Read);
            self.mark_transferred_closure(&argument.value, mode, scopes);
            self.use_expr(&argument.value, scopes, mode);
            if mode == UseMode::Give {
                let values = scopes
                    .1
                    .take(&cleanup::Slot::Temporary(argument.value.span()));
                scopes.1.put(
                    cleanup::Slot::PendingArgument { call: site, index },
                    values,
                    scopes.lexical_depth(),
                );
            }
            self.activate_place_input_borrows(&argument.value, scopes);
            if matches!(mode, UseMode::Read | UseMode::Write) {
                self.activate_borrow(&argument.value, mode, scopes);
            }
            self.activate_retained_value_borrows(&argument.value, scopes);
        }
        self.active_borrows.truncate(borrow_depth);
        for index in 0..args.len() {
            // Only after all arguments succeed does the callee own them.
            scopes
                .1
                .take(&cleanup::Slot::PendingArgument { call: site, index });
        }
    }

    fn mark_transferred_closure(&mut self, argument: &Expr, mode: UseMode, scopes: &mut Scopes) {
        if mode == UseMode::Give {
            if let Some(value) = self.prepare_retained_value(argument, scopes) {
                // A taking callee owns the environment and may retain it beyond
                // this activation. Borrow/lifetime validation remains separate.
                self.mark_closure_escape(&value, ClosureEscapeClassification::Owned);
            }
        }
    }

    fn use_call_args(
        &mut self,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        execution: CallExecution,
        scopes: &mut Scopes,
    ) {
        self.use_call_args_with_storage(site, receiver, args, signature, execution, scopes, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn use_call_args_with_storage(
        &mut self,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        execution: CallExecution,
        scopes: &mut Scopes,
        function_storage: Option<FunctionStorageBoundary<'_>>,
    ) {
        let borrow_depth = self.active_borrows.len();
        let without_call = (execution == CallExecution::Maybe).then(|| scopes.clone());
        if receiver.is_none() && signature.receiver == Some(UseMode::Give) {
            if let Some(binding) = scopes.get("this").and_then(|binding| binding.canonical_id) {
                self.use_binding("this", site, scopes, UseMode::Give);
                let values = scopes.1.take(&cleanup::Slot::Binding(binding));
                scopes.1.put(
                    cleanup::Slot::PendingCallee(site),
                    values,
                    scopes.lexical_depth(),
                );
            } else {
                self.diagnostics.push(
                    Diagnostic::new("E0474", "borrowed `$this` cannot be given away", site)
                        .with_help("a consuming parent call requires ownership of the receiver"),
                );
            }
        }
        if let Some(receiver) = receiver {
            let mode = if execution == CallExecution::Never {
                UseMode::Read
            } else {
                signature.receiver.unwrap_or(UseMode::Read)
            };
            self.check_pending_storage_access(receiver, mode, scopes);
            if mode == UseMode::Give && self.expr_returns_borrow(receiver, scopes) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0474",
                        "borrowed receiver cannot be given to a consuming method",
                        receiver.span(),
                    )
                    .with_help("call the consuming method on an independently owned value"),
                );
            }
            self.use_expr(
                receiver,
                scopes,
                if mode == UseMode::Give {
                    UseMode::Give
                } else {
                    UseMode::Read
                },
            );
            self.activate_place_input_borrows(receiver, scopes);
            if execution == CallExecution::Never {
                self.active_borrows.truncate(borrow_depth);
                return;
            }
            if mode == UseMode::Give {
                let values = self.cleanup_take_expr(receiver, scopes);
                scopes.1.put(
                    cleanup::Slot::PendingCallee(site),
                    values,
                    scopes.lexical_depth(),
                );
            } else if let Some(mode) = signature.receiver {
                self.activate_borrow(receiver, mode, scopes);
            }
            self.activate_retained_value_borrows(receiver, scopes);
        }
        // Arguments are visited in source (written) order so ownership and
        // borrow conflicts are checked over the caller-visible evaluation order
        // (decision 0098). Each argument's ownership mode, however, comes from
        // the parameter it binds to by name, not its source position.
        let bound = signature.bind_arguments(args);
        let enum_case = self.enum_case_constructions.get(&site).copied();
        if enum_case.is_some() {
            self.retained.expression_values.remove(&site);
        }
        let mut enum_result = None;
        for (index, argument) in args.iter().enumerate() {
            let arg = &argument.value;
            let param_index = bound.arg_to_param.get(index).copied().flatten();
            let mut mode = param_index.map_or(UseMode::Read, |param| {
                self.call_arg_mode(signature, param, arg)
            });
            if let (Some(boundary), Some(value)) =
                (function_storage, self.prepare_retained_value(arg, scopes))
            {
                let valid = match boundary {
                    FunctionStorageBoundary::Transport(destination) => {
                        self.validate_transported_storage(&value, arg.span(), destination)
                    }
                    FunctionStorageBoundary::Deferred { destination, title } => self
                        .reject_deferred_function_storage(&value, arg.span(), destination, title),
                };
                if !valid {
                    mode = UseMode::Read;
                }
            }
            if mode == UseMode::Write
                && param_index
                    .and_then(|param| signature.params.get(param))
                    .is_some_and(|param| !param.class_type)
            {
                self.check_writable_move_argument(arg, scopes);
            }
            if mode == UseMode::Give {
                if self.expr_returns_borrow(arg, scopes) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0474",
                            "borrowed call result cannot be given away",
                            arg.span(),
                        )
                        .with_help(
                            "pass an independently owned value to an ownership-taking parameter",
                        ),
                    );
                    self.use_expr(arg, scopes, UseMode::Read);
                    continue;
                }
                if let Some(root) = self.borrow_root_keys(arg, scopes).into_iter().find(|root| {
                    self.active_borrows
                        .iter()
                        .skip(borrow_depth)
                        .any(|borrow| borrow.root.overlaps(root))
                }) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0471",
                            format!(
                                "`{}` cannot be borrowed and given away in the same call",
                                display_borrow_root(&root)
                            ),
                            arg.span(),
                        )
                        .with_help("pass distinct owners for borrowed and ownership-taking inputs"),
                    );
                }
                for root in self.borrow_root_keys(arg, scopes) {
                    self.check_give_against_active_borrows(&root, arg.span());
                }
            }
            self.mark_transferred_closure(arg, mode, scopes);
            self.use_expr(arg, scopes, mode);
            if mode == UseMode::Give {
                let values = scopes.1.take(&cleanup::Slot::Temporary(arg.span()));
                scopes.1.put(
                    cleanup::Slot::PendingArgument { call: site, index },
                    values,
                    scopes.lexical_depth(),
                );
            }
            if let Some(value) = self.retained_value_from_expr(arg, scopes) {
                if let Some(FunctionStorageBoundary::Transport(destination)) = function_storage {
                    self.validate_transported_storage(&value, arg.span(), destination);
                }
                if let (Some(case), Some(field)) = (enum_case, param_index) {
                    let result = enum_result.get_or_insert_with(|| {
                        let mut value = retained::iterator_value(Vec::new());
                        value.enum_payloads = Some(HashMap::new());
                        value
                    });
                    result
                        .storage
                        .merge_from(&value.storage, &result.leases, &value.leases);
                    result.refresh_stored_loans();
                    result
                        .enum_payloads
                        .as_mut()
                        .expect("enum result tracks payloads")
                        .insert((case, field), value);
                }
            }
            self.activate_place_input_borrows(arg, scopes);
            if matches!(mode, UseMode::Read | UseMode::Write) {
                self.activate_borrow(arg, mode, scopes);
            }
            self.activate_retained_value_borrows(arg, scopes);
        }
        if let Some(result) = enum_result {
            self.retained.expression_values.insert(site, result);
        }
        if execution != CallExecution::Never {
            self.apply_writeback_effects(site, receiver, args, signature, scopes);
            self.validate_retained_call_inputs(receiver, args, signature, scopes);
            self.retain_pending_call_inputs(site, receiver, args, signature, None, scopes);
            self.transfer_stored_call_sources(site, receiver, args, signature, scopes);
        }
        for index in 0..args.len() {
            scopes
                .1
                .take(&cleanup::Slot::PendingArgument { call: site, index });
        }
        scopes.1.take(&cleanup::Slot::PendingCallee(site));
        if let Some(without_call) = without_call {
            let with_call = scopes.clone();
            scopes.merge_from(&without_call, &with_call);
        }
        self.active_borrows.truncate(borrow_depth);
    }

    fn method_call_execution(&self, null_safe: bool, object: &Expr) -> CallExecution {
        if !null_safe {
            return CallExecution::Always;
        }
        match self.flow_fact(object) {
            Some(Fact::Null) => CallExecution::Never,
            Some(Fact::NonNull | Fact::Exact(_) | Fact::Constructed { .. }) => {
                CallExecution::Always
            }
            None => CallExecution::Maybe,
        }
    }

    fn flow_fact(&self, expr: &Expr) -> Option<&Fact> {
        let expr = ungroup_expr(expr);
        self.flow_facts.get(&expr.span())
    }

    fn activate_borrow(&mut self, expr: &Expr, mode: UseMode, scopes: &Scopes) {
        let through = self.borrow_input_bindings(expr, scopes);
        for root in self.borrow_root_keys(expr, scopes) {
            self.activate_borrow_root(root, mode, expr.span(), &through, scopes);
        }
    }

    fn activate_retained_value_borrows(&mut self, expr: &Expr, scopes: &Scopes) {
        if let Some(value) = self.retained_value_from_expr(expr, scopes) {
            for loan in &value.leases {
                for (root, access) in loan.places() {
                    self.active_borrows.push(ActiveBorrow {
                        root: root.clone(),
                        mode: match access {
                            BorrowAccess::Readonly => UseMode::Read,
                            BorrowAccess::Writable => UseMode::Write,
                        },
                        span: loan.capture_span,
                    });
                }
            }
        }
    }

    fn activate_place_borrow(&mut self, expr: &Expr, mode: UseMode, scopes: &Scopes) {
        let through = self.borrow_input_bindings(expr, scopes);
        for root in self.borrow_root_keys(expr, scopes) {
            self.activate_borrow_root(root, mode, expr.span(), &through, scopes);
        }
    }

    fn activate_borrow_root(
        &mut self,
        root: BorrowPlace,
        mode: UseMode,
        span: Span,
        through: &[OwnershipSlotId],
        scopes: &Scopes,
    ) {
        self.check_borrow_root_access(&root, mode, span, through, scopes);
        self.active_borrows.push(ActiveBorrow { root, mode, span });
    }

    fn check_borrow_root_access(
        &mut self,
        root: &BorrowPlace,
        mode: UseMode,
        span: Span,
        through: &[OwnershipSlotId],
        scopes: &Scopes,
    ) {
        self.check_live_retained_conflict(root, mode, span, scopes);
        {
            if let Some(alias) = scopes.borrowed_from(root, through, mode) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0477",
                        format!(
                            "`{}` cannot be used as {} while `${alias}` borrows from it",
                            display_borrow_root(root),
                            if mode == UseMode::Read { "readonly" } else { "writable" }
                        ),
                        span,
                    )
                    .with_help(
                        "finish using the borrowed binding or place it in a shorter lexical block before mutating its owner",
                    ),
                );
            }
        }
        self.check_active_borrow_conflict(root, mode, span);
    }

    fn activate_place_input_borrows(&mut self, expr: &Expr, scopes: &Scopes) {
        if let Expr::PropertyAccess { object, .. } = ungroup_expr(expr) {
            if let Some(signature) = self.getter_signature(expr) {
                self.activate_nested_call_property_borrows(Some(object), &[], &signature, scopes);
                return;
            }
        }
        match expr {
            Expr::Grouped { expr, .. } | Expr::PropertyAccess { object: expr, .. } => {
                self.activate_place_input_borrows(expr, scopes);
            }
            _ => self.activate_nested_property_borrows(expr, scopes),
        }
    }

    fn activate_nested_property_borrows(&mut self, expr: &Expr, scopes: &Scopes) {
        if let Expr::PropertyAccess { object, .. } = ungroup_expr(expr) {
            if let Some(signature) = self.getter_signature(expr) {
                self.activate_nested_call_property_borrows(Some(object), &[], &signature, scopes);
                return;
            }
        }
        match expr {
            Expr::PropertyAccess { object, .. } => {
                let through = self.borrow_input_bindings(expr, scopes);
                for root in self.borrow_root_keys(expr, scopes) {
                    if !self.active_assignment_writes.contains(&root) {
                        self.activate_borrow_root(
                            root,
                            UseMode::Read,
                            expr.span(),
                            &through,
                            scopes,
                        );
                    }
                }
                self.activate_place_input_borrows(object, scopes);
            }
            Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } => {
                self.activate_nested_property_borrows(expr, scopes);
            }
            Expr::Binary {
                left,
                op: op @ (BinaryOp::And | BinaryOp::Or),
                right,
                ..
            } => {
                self.activate_nested_property_borrows(left, scopes);
                if !matches!(
                    (op, constant_bool(left)),
                    (BinaryOp::And, Some(false)) | (BinaryOp::Or, Some(true))
                ) {
                    self.activate_nested_property_borrows(right, scopes);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.activate_nested_property_borrows(left, scopes);
                self.activate_nested_property_borrows(right, scopes);
            }
            Expr::Range { start, end, .. } => {
                self.activate_nested_property_borrows(start, scopes);
                self.activate_nested_property_borrows(end, scopes);
            }
            Expr::InterpolatedString { parts, .. } => {
                for part in parts {
                    if let ast::InterpolatedStringPart::Expr(expr) = part {
                        self.activate_nested_property_borrows(expr, scopes);
                    }
                }
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.activate_nested_property_borrows(key, scopes);
                    }
                    self.activate_nested_property_borrows(&element.value, scopes);
                }
            }
            Expr::ArrayRepeat { value, count, .. } => {
                self.activate_nested_property_borrows(value, scopes);
                self.activate_nested_property_borrows(count, scopes);
            }
            Expr::FunctionCall { name, args, .. } => {
                let signature = self.signatures.get(name).cloned().unwrap_or_default();
                self.activate_nested_call_property_borrows(None, args, &signature, scopes);
            }
            Expr::New {
                class_type, args, ..
            } => {
                let signature = self
                    .constructors
                    .get(&class_type.name)
                    .cloned()
                    .unwrap_or_default();
                self.activate_nested_call_property_borrows(None, args, &signature, scopes);
            }
            Expr::MethodCall {
                object,
                method,
                args,
                null_safe,
                ..
            } => {
                let signature = self
                    .expr_class(object, scopes)
                    .and_then(|class| self.methods.get(&(class, method.clone())).cloned())
                    .unwrap_or_default();
                if self.method_call_execution(*null_safe, object) == CallExecution::Never {
                    self.activate_nested_property_borrows(object, scopes);
                } else {
                    self.activate_nested_call_property_borrows(
                        Some(object),
                        args,
                        &signature,
                        scopes,
                    );
                }
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                span,
                ..
            } => {
                let signature = self
                    .static_call_signature(*span, qualifier, method)
                    .unwrap_or_default();
                self.activate_nested_call_property_borrows(None, args, &signature, scopes);
            }
            _ => {}
        }
    }

    fn activate_nested_call_property_borrows(
        &mut self,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &Scopes,
    ) {
        if let Some(receiver) = receiver {
            self.activate_place_input_borrows(receiver, scopes);
            let result_continues_receiver_borrow = signature
                .return_borrow
                .is_some_and(|borrow| borrow.source == BorrowSource::Receiver);
            if !result_continues_receiver_borrow {
                if let Some(mode @ (UseMode::Read | UseMode::Write)) = signature.receiver {
                    self.activate_borrow(receiver, mode, scopes);
                }
            }
        }
        let bound = signature.bind_arguments(args);
        for (index, argument) in args.iter().enumerate() {
            let arg = &argument.value;
            let mode = bound
                .arg_to_param
                .get(index)
                .copied()
                .flatten()
                .map_or(UseMode::Read, |param| {
                    self.call_arg_mode(signature, param, arg)
                });
            self.activate_place_input_borrows(arg, scopes);
            let result_continues_argument_borrow = bound
                .arg_to_param
                .get(index)
                .copied()
                .flatten()
                .is_some_and(|param| {
                    signature
                        .return_borrow
                        .is_some_and(|borrow| borrow.source == BorrowSource::Parameter(param))
                });
            if matches!(mode, UseMode::Read | UseMode::Write) && !result_continues_argument_borrow {
                self.activate_borrow(arg, mode, scopes);
            }
        }
    }

    fn use_read_with_place_borrow(&mut self, expr: &Expr, scopes: &mut Scopes) {
        self.use_expr(expr, scopes, UseMode::Read);
        self.activate_place_input_borrows(expr, scopes);
        self.activate_borrow(expr, UseMode::Read, scopes);
    }

    fn check_writable_move_argument(&mut self, expr: &Expr, scopes: &Scopes) {
        if let Some((subject, span)) = self.readonly_writable_path(expr, scopes) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0479",
                    format!("readonly {subject} cannot be used as writable"),
                    span,
                )
                .with_help("make every borrowed binding and property in the path `writable` before passing it for mutation"),
            );
        }
    }

    fn readonly_writable_path(&self, expr: &Expr, scopes: &Scopes) -> Option<(String, Span)> {
        if let Some(signature) = self.getter_signature(expr) {
            return self.readonly_return_borrow_path(&signature, expr.span());
        }
        match expr {
            Expr::Grouped { expr, .. } => self.readonly_writable_path(expr, scopes),
            Expr::Index { collection, .. } => self.readonly_writable_path(collection, scopes),
            // Direct bindings are checked by `use_expr` in write mode. This
            // helper supplies the path-sensitive checks that mode cannot see.
            Expr::Variable { .. } => None,
            Expr::This { span } if !self.receiver_writable => Some(("`$this`".to_string(), *span)),
            Expr::This { .. } => None,
            Expr::PropertyAccess {
                object,
                property,
                span,
                ..
            } => self.readonly_writable_path(object, scopes).or_else(|| {
                self.expr_class(object, scopes)
                    .and_then(|class| self.properties.get(&(class, property.clone())))
                    .is_some_and(|property| {
                        !crate::types::RetainedFieldAccess::permits_write_through(
                            property.borrowed_source,
                            property.writable,
                        )
                    })
                    .then(|| (format!("property `${property}`"), *span))
            }),
            Expr::StaticMember {
                qualifier,
                member,
                span,
                ..
            } => self.qualifier_class(qualifier).and_then(|class| {
                self.static_properties
                    .get(&(class.clone(), member.clone()))
                    .is_some_and(|writable| !writable)
                    .then(|| (format!("static property `{class}::{member}`"), *span))
            }),
            Expr::FunctionCall { name, span, .. } => self
                .signatures
                .get(name)
                .and_then(|signature| self.readonly_return_borrow_path(signature, *span)),
            Expr::MethodCall {
                object,
                method,
                span,
                ..
            } => self.expr_class(object, scopes).and_then(|class| {
                self.methods
                    .get(&(class, method.clone()))
                    .and_then(|signature| self.readonly_return_borrow_path(signature, *span))
            }),
            Expr::StaticCall {
                qualifier,
                method,
                span,
                ..
            } => self.qualifier_class(qualifier).and_then(|class| {
                self.methods
                    .get(&(class, method.clone()))
                    .and_then(|signature| self.readonly_return_borrow_path(signature, *span))
            }),
            _ => None,
        }
    }

    fn readonly_return_borrow_path(
        &self,
        signature: &Signature,
        span: Span,
    ) -> Option<(String, Span)> {
        let borrow = signature
            .return_borrow
            .filter(|borrow| borrow.kind == ReturnBorrowKind::Value)?;
        (!borrow.writable).then(|| ("returned borrow".to_string(), span))
    }

    fn check_assignment_write_conflict(
        &mut self,
        root: &BorrowPlace,
        mode: UseMode,
        span: Span,
    ) -> bool {
        if !self
            .active_assignment_writes
            .iter()
            .any(|written| written.overlaps(root))
        {
            return false;
        }
        match mode {
            UseMode::Give => self.diagnostics.push(
                Diagnostic::new(
                    "E0471",
                    format!(
                        "`{}` cannot be given away while it is the destination of a property assignment",
                        display_borrow_root(root)
                    ),
                    span,
                )
                .with_help(
                    "compute the replacement without giving away the object being assigned",
                ),
            ),
            UseMode::Read | UseMode::Write => {
                let requested = if mode == UseMode::Read {
                    "readonly"
                } else {
                    "writable"
                };
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0477",
                        format!(
                            "`{}` cannot be used as {requested} while it is the destination of a property assignment",
                            display_borrow_root(root)
                        ),
                        span,
                    )
                    .with_help(
                        "finish computing the property value before using that property or its parent again",
                    ),
                );
            }
        }
        true
    }

    fn borrow_root_key(&self, expr: &Expr, scopes: &Scopes) -> Option<BorrowPlace> {
        self.borrow_source_root(expr, scopes)
            .and_then(|root| root.place_key().cloned())
    }

    fn borrow_root_keys(&self, expr: &Expr, scopes: &Scopes) -> Vec<BorrowPlace> {
        self.borrow_source_root(expr, scopes)
            .map(|root| root.places().to_vec())
            .unwrap_or_default()
    }

    fn coalesce_source_arms<'e>(
        &self,
        left: &'e Expr,
        right: &'e Expr,
    ) -> impl Iterator<Item = &'e Expr> {
        let fact = self.flow_fact(left);
        let absent = fact == Some(&Fact::Null) || matches!(ungroup_expr(left), Expr::Null { .. });
        let present = matches!(
            fact,
            Some(Fact::NonNull | Fact::Exact(_) | Fact::Constructed { .. })
        );
        [(!absent).then_some(left), (!present).then_some(right)]
            .into_iter()
            .flatten()
    }

    fn creates_owned_interface_temporary(&self, expr: &Expr) -> bool {
        self.interface_conversion_types.contains_key(&expr.span())
            && self.resolved_type(expr).is_some_and(|ty| {
                !matches!(ty, ResolvedType::Null | ResolvedType::Unsupported)
                    && !resolved_type_is_move_type(ty, &self.move_enum_names)
                    && !resolved_type_requires_conservative_move(ty)
            })
    }

    fn borrow_input_bindings(&self, expr: &Expr, scopes: &Scopes) -> Vec<OwnershipSlotId> {
        if self.creates_owned_interface_temporary(expr) {
            return Vec::new();
        }
        if let Some(sources) = self
            .conditional_borrow_sources
            .get(&ungroup_expr(expr).span())
        {
            return sources
                .bindings
                .iter()
                .filter_map(|id| id.and_then(|id| scopes.get_by_canonical(id)))
                .map(|(_, binding)| binding.id)
                .collect();
        }
        if let Some((signature, receiver, args)) = self.retained_call(expr, scopes) {
            return signature
                .return_borrow
                .and_then(|borrow| self.call_borrow_source_expr(borrow, receiver, &signature, args))
                .map(|source| self.borrow_input_bindings(source, scopes))
                .unwrap_or_default();
        }
        match ungroup_expr(expr) {
            Expr::Variable { name, .. } => scopes
                .get(name)
                .map(|binding| binding.id)
                .into_iter()
                .collect(),
            Expr::PropertyAccess { object, .. } => self.borrow_input_bindings(object, scopes),
            Expr::Index { collection, .. } => self.borrow_input_bindings(collection, scopes),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => {
                let mut inputs = Vec::new();
                for arm in self.coalesce_source_arms(left, right) {
                    for input in self.borrow_input_bindings(arm, scopes) {
                        if !inputs.contains(&input) {
                            inputs.push(input);
                        }
                    }
                }
                inputs
            }
            _ => Vec::new(),
        }
    }

    fn borrow_source_root(&self, expr: &Expr, scopes: &Scopes) -> Option<BorrowSourceRoot> {
        // Copy erasure owns a snapshot. Its carrier dies with the statement;
        // a returned borrow cannot use the original Copy binding as its owner.
        if self.creates_owned_interface_temporary(expr) {
            return None;
        }
        if let Some(sources) = self
            .conditional_borrow_sources
            .get(&ungroup_expr(expr).span())
        {
            let mut places = Vec::new();
            for source in &sources.bindings {
                let root = match source {
                    Some(id) => {
                        let (name, binding) = scopes.get_by_canonical(*id)?;
                        binding.borrow_root.clone().unwrap_or_else(|| {
                            BorrowSourceRoot::Place(binding_identity_key(binding.id, name).into())
                        })
                    }
                    None => BorrowSourceRoot::Place("$this".into()),
                };
                places.extend_from_slice(root.places());
            }
            let root = BorrowSourceRoot::from_places(places);
            return Some(if sources.origins.kind == ReturnBorrowKind::Value {
                root.map_places(BorrowPlace::opaque)
            } else {
                root
            });
        }
        if let Some((signature, receiver, args)) = self.retained_call(expr, scopes) {
            return signature.return_borrow.and_then(|borrow| {
                self.call_borrow_source_root(borrow, receiver, &signature, args, scopes)
                    .map(|root| {
                        if borrow.kind == ReturnBorrowKind::Value {
                            root.map_places(BorrowPlace::opaque)
                        } else {
                            root
                        }
                    })
            });
        }
        match expr {
            Expr::Null { .. } => Some(BorrowSourceRoot::ImmutableNull),
            Expr::This { .. } if self.receiver_class.is_some() => {
                Some(BorrowSourceRoot::Place("$this".into()))
            }
            Expr::Variable { name, .. } => scopes.get(name).map(|binding| {
                binding.borrow_root.clone().unwrap_or_else(|| {
                    BorrowSourceRoot::Place(binding_identity_key(binding.id, name).into())
                })
            }),
            Expr::Grouped { expr: object, .. } => self.borrow_source_root(object, scopes),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => {
                let mut places = Vec::new();
                for arm in self.coalesce_source_arms(left, right) {
                    places.extend_from_slice(self.borrow_source_root(arm, scopes)?.places());
                }
                Some(BorrowSourceRoot::from_places(places))
            }
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let root = self.borrow_source_root(object, scopes)?;
                let stored = self
                    .resolved_type(object)
                    .and_then(resolved_type_class)
                    .and_then(|class| {
                        self.retained
                            .properties
                            .get(&(class.to_owned(), property.clone()))
                    });
                // A retained field's slot is its own stored path: the caller's
                // source is reachable only through it, and construction keeps
                // writable sources exclusive, so sibling fields stay disjoint.
                Some(root.map_places(|place| match stored {
                    Some((id, _)) => place.project(*id),
                    None => place.opaque(),
                }))
            }
            Expr::Index { collection, .. } => self
                .borrow_source_root(collection, scopes)
                .map(|root| root.map_places(BorrowPlace::opaque)),
            Expr::CallableCall { args, span, .. } => {
                let call = self.callable_value_calls.get(span)?;
                let function = non_null_function_type(&call.function_type)?;
                let borrow = function.return_borrow?;
                let crate::types::FunctionBorrowSource::Parameter(index) = borrow.source;
                self.borrow_source_root(&args.get(index)?.value, scopes)
                    .filter(|root| !borrow.writable || *root != BorrowSourceRoot::ImmutableNull)
            }
            Expr::StaticMember {
                qualifier, member, ..
            } => self.qualifier_class(qualifier).and_then(|class| {
                self.static_properties
                    .contains_key(&(class.clone(), member.clone()))
                    .then(|| BorrowSourceRoot::Place(format!("static:{class}::{member}").into()))
            }),
            Expr::FunctionCall { name, args, .. } => {
                let signature = self.signatures.get(name)?;
                let borrow = signature.return_borrow?;
                self.call_borrow_source_root(borrow, None, signature, args, scopes)
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                if self
                    .expr_collection_info(object, scopes)
                    .is_some_and(|collection| collection.family.borrows_element(method))
                {
                    return self.borrow_source_root(object, scopes);
                }
                let class = self.expr_class(object, scopes)?;
                let signature = self.methods.get(&(class, method.clone()))?;
                let borrow = signature.return_borrow?;
                self.call_borrow_source_root(borrow, Some(object), signature, args, scopes)
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                ..
            } => {
                let class = self.qualifier_class(qualifier)?;
                let signature = self.methods.get(&(class, method.clone()))?;
                let borrow = signature.return_borrow?;
                self.call_borrow_source_root(borrow, None, signature, args, scopes)
            }
            _ => None,
        }
    }

    fn assignment_place_key(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
        if self.getter_signature(expr).is_some() {
            return None;
        }
        match expr {
            Expr::This { .. } if self.receiver_class.is_some() => Some("$this".to_string()),
            Expr::Variable { name, .. } => scopes
                .get(name)
                .map(|binding| binding_identity_key(binding.id, name)),
            Expr::Grouped { expr, .. } => self.assignment_place_key(expr, scopes),
            Expr::PropertyAccess {
                object, property, ..
            } => self
                .assignment_place_key(object, scopes)
                .map(|object| format!("{object}->{property}")),
            Expr::StaticMember { .. } => {
                self.borrow_root_key(expr, scopes).map(|place| place.owner)
            }
            _ => None,
        }
    }

    fn call_borrow_source_root(
        &self,
        borrow: ReturnBorrow,
        receiver: Option<&Expr>,
        signature: &Signature,
        args: &[Argument],
        scopes: &Scopes,
    ) -> Option<BorrowSourceRoot> {
        let source = self.call_borrow_source_expr(borrow, receiver, signature, args)?;
        self.borrow_source_root(source, scopes)
            .filter(|root| !borrow.writable || *root != BorrowSourceRoot::ImmutableNull)
    }

    fn check_active_borrow_conflict(&mut self, root: &BorrowPlace, mode: UseMode, span: Span) {
        if mode == UseMode::Give {
            self.check_give_against_active_borrows(root, span);
            return;
        }
        let Some(existing) = self
            .active_borrows
            .iter()
            .rev()
            .find(|borrow| borrow.root.overlaps(root) && borrow_modes_conflict(borrow.mode, mode))
            .cloned()
        else {
            return;
        };
        let existing_span = existing.span;
        let requested = match mode {
            UseMode::Read => "readonly",
            UseMode::Write => "writable",
            UseMode::Give => unreachable!("handled above"),
        };
        let existing = match existing.mode {
            UseMode::Read => "readonly",
            UseMode::Write => "writable",
            UseMode::Give => unreachable!("active borrow cannot be a give"),
        };
        let root_display = display_borrow_root(root);
        self.diagnostics.push(
            Diagnostic::new(
                "E0477",
                format!(
                    "`{root_display}` cannot be used as {requested} here because an earlier live access uses it as {existing}"
                ),
                span,
            )
            .with_title("Conflicting Access")
            .with_primary_label("This Access Conflicts With an Earlier Access")
            .with_explanation(
                "Writable access must be exclusive for the complete duration of the earlier access.",
            )
            .with_related(existing_span, "Earlier Access Starts Here")
            .with_help("Finish the earlier access before taking the conflicting writable access."),
        );
    }

    fn check_give_against_active_borrows(&mut self, root: &BorrowPlace, span: Span) {
        if let Some(existing) = self
            .active_borrows
            .iter()
            .rev()
            .find(|borrow| borrow.root.overlaps(root))
            .cloned()
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0471",
                    format!(
                        "`{}` cannot be borrowed and given away in the same call",
                        display_borrow_root(root)
                    ),
                    span,
                )
                .with_help(format!(
                    "the earlier use at bytes {}..{} must finish before ownership is given away",
                    existing.span.start, existing.span.end
                )),
            );
        }
    }

    fn use_owned_expression(&mut self, expr: &Expr, scopes: &mut Scopes) -> UseMode {
        if self.reject_borrowed_result(
            expr,
            scopes,
            "borrowed result cannot be stored in an owning collection",
            "store an independently owned value in the collection",
        ) {
            self.use_expr(expr, scopes, UseMode::Read);
            return UseMode::Read;
        }
        let mode = if self.expr_is_move_value(expr, scopes) {
            UseMode::Give
        } else {
            UseMode::Read
        };
        self.use_expr(expr, scopes, mode);
        mode
    }

    fn reject_borrowed_result(
        &mut self,
        expr: &Expr,
        scopes: &Scopes,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> bool {
        if !self.expr_returns_borrow(expr, scopes) {
            return false;
        }
        self.diagnostics
            .push(Diagnostic::new("E0478", message, expr.span()).with_help(help));
        true
    }

    fn reject_returned_temporary_borrow(&mut self, expr: &Expr, scopes: &Scopes) -> bool {
        if !self.expr_returns_borrow(expr, scopes)
            || self.borrow_source_root(expr, scopes).is_some()
        {
            return false;
        }
        self.diagnostics.push(
            Diagnostic::new(
                "E0478",
                "borrowed result from a temporary cannot escape through a return",
                expr.span(),
            )
            .with_help("store an owned value before borrowing it, and keep every returned borrow within that owner's lifetime"),
        );
        true
    }

    fn expr_is_move_value(&self, expr: &Expr, scopes: &Scopes) -> bool {
        if let Some(ty) = self.resolved_type(expr) {
            if !resolved_type_requires_conservative_move(ty) {
                return resolved_type_is_move_type(ty, &self.move_enum_names);
            }
        }
        match expr {
            Expr::Variable { name, .. } => scopes.get(name).is_some(),
            Expr::Grouped { expr, .. } => self.expr_is_move_value(expr, scopes),
            Expr::Array { .. } => true,
            Expr::ArrayRepeat { .. } => true,
            Expr::New { class_type, .. } => self.classes.contains(&class_type.name),
            Expr::FunctionCall { name, .. } => {
                Builtin::from_name(name).is_some_and(Builtin::returns_owned_bytes)
                    || self
                        .signatures
                        .get(name)
                        .is_some_and(|signature| signature.returns_move_type)
            }
            Expr::MethodCall { object, method, .. } => {
                if let Some(collection) = self.expr_collection_info(object, scopes) {
                    return matches!(
                        (collection.family, method.as_str()),
                        (CollectionFamily::List, "removeAt" | "pop")
                            | (CollectionFamily::Dictionary, "remove")
                            | (CollectionFamily::PriorityQueue, "pop")
                            | (CollectionFamily::Deque, "popFront" | "popBack")
                    ) && collection.value_move
                        || matches!(
                            (collection.family, method.as_str()),
                            (CollectionFamily::Bytes, "toArray")
                        )
                        || matches!(
                            (collection.family, method.as_str()),
                            (CollectionFamily::Set, "union" | "intersect" | "difference")
                        );
                }
                let Some(class) = self.expr_class(object, scopes) else {
                    return false;
                };
                self.methods
                    .get(&(class, method.clone()))
                    .is_some_and(|signature| signature.returns_move_type)
            }
            Expr::StaticCall {
                qualifier, method, ..
            } => {
                if matches!(
                    qualifier,
                    ast::StaticQualifier::Class(class_name)
                        if (class_name == "Set" && method == "from")
                            || (class_name == "Bytes"
                                && matches!(method.as_str(), "fromArray" | "allocate"))
                ) {
                    return true;
                }
                self.qualifier_class(qualifier)
                    .and_then(|class_name| self.methods.get(&(class_name, method.clone())))
                    .is_some_and(|signature| signature.returns_move_type)
            }
            Expr::PropertyAccess {
                object, property, ..
            } => {
                if let Some(collection) = self.expr_collection_info(object, scopes) {
                    return collection.family.borrows_element(property) && collection.value_move;
                }
                let Some(class) = self.expr_class(object, scopes) else {
                    return false;
                };
                self.properties
                    .get(&(class, property.clone()))
                    .is_some_and(|property| property.move_type)
            }
            Expr::This { .. } => self.receiver_class.is_some(),
            Expr::Index { collection, .. } => self
                .expr_collection_info(collection, scopes)
                .is_some_and(|collection| collection.value_move),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => self.expr_is_move_value(left, scopes) || self.expr_is_move_value(right, scopes),
            _ => false,
        }
    }

    fn expr_is_non_transferable_property(&self, expr: &Expr, scopes: &Scopes) -> bool {
        if let Some(signature) = self.getter_signature(expr) {
            return signature
                .return_borrow
                .is_some_and(|borrow| borrow.kind == ReturnBorrowKind::Value);
        }
        let Expr::PropertyAccess {
            object, property, ..
        } = expr
        else {
            return false;
        };
        if property == "referencedValue"
            && self.resolved_type(object).is_some_and(|ty| {
                resolved_shared_handle_kind(ty)
                    == Some(crate::types::SharedHandleKind::SharedReference)
            })
        {
            return true;
        }
        if let Some(collection) = self.expr_collection_info(object, scopes) {
            return collection.family.borrows_element(property) && collection.value_move;
        }
        let Some(class) = self.expr_class(object, scopes) else {
            return false;
        };
        self.properties
            .get(&(class, property.clone()))
            .is_some_and(|property| property.move_type)
    }

    fn borrowed_carrier_alias(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match ungroup_expr(expr) {
            Expr::Variable { name, .. } => scopes.get(name).is_some_and(|binding| {
                (binding.function_type.is_some()
                    || binding.class.is_some()
                    || binding.collection.is_some()
                    || binding.mixed
                    || self.resolved_type(expr).is_some_and(|ty| {
                        resolved_type_is_move_type(ty, &self.move_enum_names)
                            || resolved_type_requires_conservative_move(ty)
                    }))
                    && matches!(binding.state, State::Borrowed | State::BorrowedOrOwned)
            }),
            _ => false,
        }
    }

    fn expr_returns_borrow(&self, expr: &Expr, scopes: &Scopes) -> bool {
        // Return-borrow inference is syntactic: it treats a field read as a
        // borrow of its owner. A scalar or string is copied out and can never
        // be a loan, whatever produced it.
        if self.resolved_type(expr).is_some_and(|ty| {
            matches!(
                ty,
                crate::types::ResolvedType::Integer(_)
                    | crate::types::ResolvedType::Float(_)
                    | crate::types::ResolvedType::Bool
                    | crate::types::ResolvedType::String
            )
        }) {
            return false;
        }
        match expr {
            Expr::Grouped { expr, .. } => self.expr_returns_borrow(expr, scopes),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => self.coalesce_source_arms(left, right).any(|arm| {
                self.expr_returns_borrow(arm, scopes) || self.borrowed_carrier_alias(arm, scopes)
            }),
            _ => {
                self.expr_has_return_borrow_contract(expr, scopes)
                    && self.borrow_source_root(expr, scopes)
                        != Some(BorrowSourceRoot::ImmutableNull)
            }
        }
    }

    fn borrowed_value_is_writable(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match ungroup_expr(expr) {
            Expr::MethodCall { object, method, .. }
                if self
                    .expr_collection_info(object, scopes)
                    .is_some_and(|collection| {
                        collection.value_move && collection.family.borrows_element(method)
                    }) =>
            {
                self.readonly_writable_path(object, scopes).is_none()
                    && self.writable_place_root(object, scopes)
            }
            Expr::Index { .. } | Expr::PropertyAccess { .. }
                if self.getter_signature(expr).is_none() =>
            {
                self.readonly_writable_path(expr, scopes).is_none()
                    && self.writable_place_root(expr, scopes)
            }
            Expr::Variable { name, .. } => scopes.get(name).is_some_and(|binding| binding.writable),
            Expr::Null { .. } => false,
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => self
                .coalesce_source_arms(left, right)
                .all(|arm| self.borrowed_value_is_writable(arm, scopes)),
            _ => {
                self.expr_has_return_borrow_contract(expr, scopes)
                    && self.function_result_borrow_is_writable(expr, scopes)
            }
        }
    }

    fn writable_place_root(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match ungroup_expr(expr) {
            Expr::Variable { name, .. } => scopes.get(name).is_some_and(|binding| binding.writable),
            Expr::This { .. } => self.receiver_writable,
            Expr::Index { collection, .. } => self.writable_place_root(collection, scopes),
            Expr::PropertyAccess { object, .. } => self.writable_place_root(object, scopes),
            Expr::StaticMember {
                qualifier, member, ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class| self.static_properties.get(&(class, member.clone())))
                .copied()
                .unwrap_or(false),
            _ => {
                !self.borrow_root_keys(expr, scopes).is_empty()
                    && self.function_result_borrow_is_writable(expr, scopes)
            }
        }
    }

    fn expr_has_return_borrow_contract(&self, expr: &Expr, scopes: &Scopes) -> bool {
        self.expr_has_return_contract(expr, scopes, Some(ReturnBorrowKind::Value))
    }

    fn expr_has_return_contract(
        &self,
        expr: &Expr,
        scopes: &Scopes,
        kind: Option<ReturnBorrowKind>,
    ) -> bool {
        let matches_kind = |borrow: ReturnBorrow| kind.is_none_or(|kind| borrow.kind == kind);
        if let Some(sources) = self
            .conditional_borrow_sources
            .get(&ungroup_expr(expr).span())
        {
            return sources.origins.borrowed
                && kind.is_none_or(|kind| sources.origins.kind == kind);
        }
        if let Some((signature, _, _)) = self.retained_call(expr, scopes) {
            return signature.return_borrow.is_some_and(matches_kind);
        }
        match expr {
            Expr::Grouped { expr, .. } => self.expr_has_return_contract(expr, scopes, kind),
            Expr::FunctionCall { name, .. } => self
                .signatures
                .get(name)
                .is_some_and(|signature| signature.return_borrow.is_some_and(matches_kind)),
            Expr::MethodCall { object, method, .. } => {
                if let Some(collection) = self.expr_collection_info(object, scopes) {
                    return collection.value_move && collection.family.borrows_element(method);
                }
                let Some(class) = self.expr_class(object, scopes) else {
                    return false;
                };
                self.methods
                    .get(&(class, method.clone()))
                    .is_some_and(|signature| signature.return_borrow.is_some_and(matches_kind))
            }
            Expr::StaticCall {
                qualifier, method, ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class| self.methods.get(&(class, method.clone())))
                .is_some_and(|signature| signature.return_borrow.is_some_and(matches_kind)),
            Expr::CallableCall { span, .. } => self
                .callable_value_calls
                .get(span)
                .and_then(|call| non_null_function_type(&call.function_type))
                .is_some_and(|function| {
                    function
                        .return_borrow
                        .is_some_and(|borrow| kind.is_none_or(|kind| borrow.kind == kind))
                }),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => {
                self.expr_has_return_contract(left, scopes, kind)
                    || self.expr_has_return_contract(right, scopes, kind)
            }
            Expr::Index { collection, .. } => self
                .expr_collection_info(collection, scopes)
                .is_some_and(|collection| collection.value_move && !collection.value_mixed),
            Expr::PropertyAccess {
                object, property, ..
            } => self
                .expr_collection_info(object, scopes)
                .is_some_and(|collection| {
                    collection.value_move && collection.family.borrows_element(property)
                }),
            _ => false,
        }
    }

    fn expr_is_mixed_collection_index(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => self.expr_is_mixed_collection_index(expr, scopes),
            Expr::Index { collection, .. } => self
                .expr_collection_info(collection, scopes)
                .is_some_and(|collection| collection.value_mixed),
            _ => false,
        }
    }

    fn expr_is_mixed_value(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match expr {
            Expr::Variable { name, .. } => scopes.get(name).is_some_and(|binding| binding.mixed),
            Expr::Grouped { expr, .. } => self.expr_is_mixed_value(expr, scopes),
            Expr::Index { collection, .. } => self
                .expr_collection_info(collection, scopes)
                .is_some_and(|collection| collection.value_mixed),
            _ => false,
        }
    }

    fn assignment_property_info<'a>(
        &'a self,
        target: &Expr,
        scopes: &Scopes,
    ) -> Option<&'a PropertyInfo> {
        match target {
            Expr::Grouped { expr, .. } => self.assignment_property_info(expr, scopes),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let class = self.expr_class(object, scopes)?;
                self.properties.get(&(class, property.clone()))
            }
            _ => None,
        }
    }

    fn expr_class(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
        if let Some(class) = self.resolved_type(expr).and_then(resolved_type_class) {
            return Some(class.to_string());
        }
        match expr {
            Expr::Variable { name, .. } => {
                scopes.get(name).and_then(|binding| binding.class.clone())
            }
            Expr::New { class_type, .. } if self.classes.contains(&class_type.name) => {
                Some(class_type.name.clone())
            }
            Expr::FunctionCall { name, .. } => self
                .signatures
                .get(name)
                .and_then(|signature| signature.returns.clone()),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                if let Some(class) = self
                    .expr_collection_info(object, scopes)
                    .and_then(|collection| collection.value_class)
                    .filter(|_| matches!(property.as_str(), "first" | "last"))
                {
                    return Some(class);
                }
                let object_class = self.expr_class(object, scopes)?;
                self.properties
                    .get(&(object_class, property.clone()))
                    .and_then(|property| property.class.clone())
            }
            Expr::MethodCall { object, method, .. } => {
                if let Some(class) =
                    self.expr_collection_info(object, scopes)
                        .and_then(|collection| {
                            matches!(
                                (collection.family, method.as_str()),
                                (CollectionFamily::List, "removeAt" | "pop")
                                    | (CollectionFamily::Dictionary, "get" | "remove")
                            )
                            .then_some(collection.value_class)
                            .flatten()
                        })
                {
                    return Some(class);
                }
                let object_class = self.expr_class(object, scopes)?;
                self.methods
                    .get(&(object_class, method.clone()))
                    .and_then(|signature| signature.returns.clone())
            }
            Expr::StaticCall {
                qualifier, method, ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class_name| self.methods.get(&(class_name, method.clone())))
                .and_then(|signature| signature.returns.clone()),
            Expr::This { .. } => self.receiver_class.clone(),
            Expr::Index { collection, .. } => self
                .expr_collection_info(collection, scopes)
                .and_then(|collection| collection.value_class),
            Expr::Grouped { expr, .. } => self.expr_class(expr, scopes),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => match (
                self.expr_class(left, scopes),
                self.expr_class(right, scopes),
            ) {
                (Some(left), Some(right)) if left == right => Some(left),
                (Some(class), None) | (None, Some(class)) => Some(class),
                _ => None,
            },
            _ => None,
        }
    }

    fn expr_collection_info(&self, expr: &Expr, scopes: &Scopes) -> Option<CollectionInfo> {
        if let Some(collection) = self
            .resolved_type(expr)
            .and_then(|ty| resolved_collection_info(ty, &self.move_enum_names))
        {
            return Some(collection);
        }
        match expr {
            Expr::Variable { name, .. } => scopes
                .get(name)
                .and_then(|binding| binding.collection.clone()),
            Expr::Array { elements, .. } => {
                let value = elements.first().map(|entry| &entry.value);
                Some(CollectionInfo {
                    family: if elements.iter().any(|entry| entry.key.is_some()) {
                        CollectionFamily::Dictionary
                    } else {
                        CollectionFamily::List
                    },
                    value_move: value.is_some_and(|value| self.expr_is_move_value(value, scopes)),
                    value_mixed: value.is_some_and(|value| self.expr_is_mixed_value(value, scopes)),
                    value_class: value.and_then(|value| self.expr_class(value, scopes)),
                    value_collection: value
                        .and_then(|value| self.expr_collection_info(value, scopes))
                        .map(Box::new),
                })
            }
            Expr::ArrayRepeat { value, .. } => Some(CollectionInfo {
                family: CollectionFamily::List,
                value_move: self.expr_is_move_value(value, scopes),
                value_mixed: self.expr_is_mixed_value(value, scopes),
                value_class: self.expr_class(value, scopes),
                value_collection: self.expr_collection_info(value, scopes).map(Box::new),
            }),
            Expr::FunctionCall { name, .. } => {
                if Builtin::from_name(name).is_some_and(Builtin::returns_owned_bytes) {
                    return Some(bytes_collection_info());
                }
                self.signatures
                    .get(name)
                    .and_then(|signature| signature.returns_collection.clone())
            }
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let class = self.expr_class(object, scopes)?;
                self.properties
                    .get(&(class, property.clone()))
                    .and_then(|property| property.collection.clone())
            }
            Expr::MethodCall { object, method, .. }
                if matches!(method.as_str(), "union" | "intersect" | "difference") =>
            {
                let info = self.expr_collection_info(object, scopes)?;
                (info.family == CollectionFamily::Set).then_some(info)
            }
            Expr::MethodCall { object, method, .. } if method == "toArray" => {
                let info = self.expr_collection_info(object, scopes)?;
                (info.family == CollectionFamily::Bytes).then_some(byte_array_collection_info())
            }
            Expr::StaticCall {
                qualifier: ast::StaticQualifier::Class(class_name),
                method,
                args,
                ..
            } if class_name == "Set" && method == "from" => {
                let source = args
                    .first()
                    .and_then(|arg| self.expr_collection_info(&arg.value, scopes));
                Some(CollectionInfo {
                    family: CollectionFamily::Set,
                    value_move: source.as_ref().is_some_and(|info| info.value_move),
                    value_mixed: source.as_ref().is_some_and(|info| info.value_mixed),
                    value_class: source.as_ref().and_then(|info| info.value_class.clone()),
                    value_collection: source.and_then(|info| info.value_collection.clone()),
                })
            }
            Expr::StaticCall {
                qualifier: ast::StaticQualifier::Class(class_name),
                method,
                ..
            } if class_name == "Bytes" && matches!(method.as_str(), "fromArray" | "allocate") => {
                Some(bytes_collection_info())
            }
            Expr::Index { collection, .. } => self
                .expr_collection_info(collection, scopes)
                .and_then(|info| info.value_collection.map(|nested| *nested)),
            Expr::Grouped { expr, .. } => self.expr_collection_info(expr, scopes),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => {
                let left = self.expr_collection_info(left, scopes);
                let right = self.expr_collection_info(right, scopes);
                (left == right).then_some(left).flatten()
            }
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn use_collection_call(
        &mut self,
        collection: CollectionFamily,
        object: &Expr,
        method: &str,
        args: &[Argument],
        span: Span,
        execution: CallExecution,
        scopes: &mut Scopes,
    ) {
        let mutating = matches!(
            (collection, method),
            (
                CollectionFamily::List,
                "add" | "insertAt" | "remove" | "removeAt" | "pop" | "clear"
            ) | (CollectionFamily::Dictionary, "set" | "remove" | "clear")
                | (CollectionFamily::Set, "add" | "remove" | "clear")
                | (CollectionFamily::PriorityQueue, "push" | "pop" | "clear")
                | (
                    CollectionFamily::Deque,
                    "pushFront" | "pushBack" | "popFront" | "popBack" | "clear"
                )
        );
        self.use_expr(
            object,
            scopes,
            if mutating {
                UseMode::Write
            } else {
                UseMode::Read
            },
        );
        if execution == CallExecution::Never {
            return;
        }
        let without_call = (execution == CallExecution::Maybe).then(|| scopes.clone());
        if let Some(plan) = self.list_algorithm_calls.get(&span).cloned() {
            let borrow_depth = self.active_borrows.len();
            self.activate_place_input_borrows(object, scopes);
            self.activate_borrow(object, UseMode::Read, scopes);
            for (index, argument) in args.iter().enumerate() {
                let is_initial =
                    plan.kind == crate::semantics::ListAlgorithmKind::Reduce && index == 0;
                if is_initial {
                    self.use_owned_expression(&argument.value, scopes);
                    let values = self.cleanup_take_expr(&argument.value, scopes);
                    scopes.1.put(
                        cleanup::Slot::PendingArgument { call: span, index },
                        values,
                        scopes.lexical_depth(),
                    );
                    continue;
                }
                let mode = match plan.callback_access {
                    crate::semantics::ListCallbackAccess::Readonly => UseMode::Read,
                    crate::semantics::ListCallbackAccess::Writable => UseMode::Write,
                };
                self.use_expr(&argument.value, scopes, mode);
            }
            // The algorithm, not its caller, owns the accumulator/partial
            // output while callbacks execute. A checked callback failure drops
            // that owner; successful completion transfers it as the result.
            let result = if plan.kind == crate::semantics::ListAlgorithmKind::Reduce {
                if let Some(initial) = args.first() {
                    self.cleanup_analysis
                        .value_flows
                        .insert((initial.value.span(), span));
                }
                scopes.1.take(&cleanup::Slot::PendingArgument {
                    call: span,
                    index: 0,
                })
            } else {
                let element_source = if plan.kind == crate::semantics::ListAlgorithmKind::Map {
                    cleanup::Source::CallbackReturn {
                        callback: plan.callback_span,
                    }
                } else {
                    cleanup::Source::CollectionElement {
                        collection: object.span(),
                        key: false,
                    }
                };
                self.cleanup_analysis.source_flows.insert((
                    element_source,
                    cleanup::Source::CollectionElement {
                        collection: span,
                        key: false,
                    },
                ));
                let contents = match &plan.result_type {
                    ResolvedType::List(element) => {
                        self.cleanup_acquire_value(element_source, *element.clone(), HashSet::new())
                    }
                    _ => HashSet::new(),
                };
                self.cleanup_acquire_value(
                    cleanup::Source::Expression(span),
                    plan.result_type.clone(),
                    contents,
                )
            };
            scopes.1.put(
                cleanup::Slot::Temporary(span),
                result,
                scopes.lexical_depth(),
            );
            self.record_exceptional_exits(span, scopes);
            self.active_borrows.truncate(borrow_depth);
            if let Some(without_call) = without_call {
                let with_call = scopes.clone();
                scopes.merge_from(&without_call, &with_call);
            }
            return;
        }
        let mut moved_arguments = Vec::new();
        let mut stored_values = Vec::new();
        for (index, argument) in args.iter().enumerate() {
            let moves_in = collection.ingested_arguments(method).contains(&index);
            if moves_in {
                let valid_function_storage = self
                    .prepare_retained_value(&argument.value, scopes)
                    .as_ref()
                    .is_none_or(|value| {
                        self.validate_collection_destination(
                            object,
                            value,
                            argument.value.span(),
                            scopes,
                        )
                    });
                if valid_function_storage {
                    self.use_owned_expression(&argument.value, scopes);
                    if let Some(value) = self.retained_value_from_expr(&argument.value, scopes) {
                        stored_values.push(value);
                    }
                    let values = self.cleanup_take_expr(&argument.value, scopes);
                    scopes.1.put(
                        cleanup::Slot::PendingArgument { call: span, index },
                        values,
                        scopes.lexical_depth(),
                    );
                    moved_arguments.push(index);
                } else {
                    self.use_expr(&argument.value, scopes, UseMode::Read);
                }
            } else {
                self.use_expr(&argument.value, scopes, UseMode::Read);
            }
        }
        // Lookup, hashing and comparison run before ownership is installed in
        // the collection. Pending arguments therefore remain live on failure.
        self.record_exceptional_exits(span, scopes);
        for value in stored_values {
            self.store_collection_sources(object, &value, span, scopes);
        }
        if mutating && method == "clear" {
            self.clear_collection_sources(object, scopes);
        }
        let roots = self.cleanup_collection_roots(object, scopes);
        for index in moved_arguments {
            let values = scopes
                .1
                .take(&cleanup::Slot::PendingArgument { call: span, index });
            let key = collection == CollectionFamily::Dictionary && index == 0;
            self.cleanup_analysis
                .value_flows
                .insert((args[index].value.span(), object.span()));
            self.cleanup_analysis.source_flows.insert((
                cleanup::Source::Expression(args[index].value.span()),
                cleanup::Source::CollectionElement {
                    collection: object.span(),
                    key,
                },
            ));
            for root in &roots {
                if let Some(value) = self.cleanup_analysis.values.get_mut(root) {
                    value.contents.extend(&values);
                }
            }
            if (collection == CollectionFamily::Set && method == "add")
                || (collection == CollectionFamily::Dictionary && method == "set" && key)
            {
                // Duplicate insertion keeps the stored element/key and drops
                // the newly supplied owner. These are conditional releases.
                self.cleanup_release(span, values, cleanup::Cause::Replacement);
            }
        }
        let receiver_type = self.resolved_type(object).cloned();
        let receiver_type = match &receiver_type {
            Some(ResolvedType::Nullable(inner)) => Some(inner.as_ref()),
            other => other.as_ref(),
        };
        let (key_type, value_type) = match receiver_type {
            Some(
                ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value),
            ) => (Some(*key.clone()), Some(*value.clone())),
            Some(
                ResolvedType::TypedArray(value)
                | ResolvedType::List(value)
                | ResolvedType::Set(value)
                | ResolvedType::SortedSet(value)
                | ResolvedType::PriorityQueue(value)
                | ResolvedType::Deque(value),
            ) => (None, Some(*value.clone())),
            _ => (None, None),
        };
        let removes_value = matches!(
            (collection, method),
            (CollectionFamily::List | CollectionFamily::Set, "remove")
        );
        let replaces_value = collection == CollectionFamily::Dictionary && method == "set";
        let clears = mutating && method == "clear";
        let removes_key = collection == CollectionFamily::Dictionary && method == "remove";
        for (key, ty, released) in [
            (true, key_type, clears || removes_key),
            (false, value_type, clears || removes_value || replaces_value),
        ] {
            if released {
                if let Some(ty) = ty {
                    let values = self.cleanup_acquire_value(
                        cleanup::Source::CollectionElement {
                            collection: object.span(),
                            key,
                        },
                        ty,
                        HashSet::new(),
                    );
                    self.cleanup_release(span, values, cleanup::Cause::Replacement);
                }
            }
        }
        if collection.removes_element(method) {
            self.cleanup_analysis.source_flows.insert((
                cleanup::Source::CollectionElement {
                    collection: object.span(),
                    key: false,
                },
                cleanup::Source::Expression(span),
            ));
            if let Some(ty) = self.resolved_types.get(&span).cloned() {
                let result = self.cleanup_acquire_value(
                    cleanup::Source::CollectionElement {
                        collection: object.span(),
                        key: false,
                    },
                    ty,
                    HashSet::new(),
                );
                scopes.1.put(
                    cleanup::Slot::Temporary(span),
                    result,
                    scopes.lexical_depth(),
                );
            }
        }
        if let Some(without_call) = without_call {
            let with_call = scopes.clone();
            scopes.merge_from(&without_call, &with_call);
        }
    }

    fn qualifier_class(&self, qualifier: &ast::StaticQualifier) -> Option<String> {
        match qualifier {
            ast::StaticQualifier::Class(name) => Some(name.clone()),
            ast::StaticQualifier::SelfType => self.receiver_class.clone(),
            ast::StaticQualifier::Parent | ast::StaticQualifier::InvalidStatic => None,
        }
    }

    fn static_call_signature(
        &self,
        site: Span,
        qualifier: &ast::StaticQualifier,
        method: &str,
    ) -> Option<Signature> {
        if let (Some(case), Some(ResolvedType::Enum(ty))) = (
            self.enum_case_constructions.get(&site),
            self.resolved_types.get(&site),
        ) {
            if let Some(definition) = self.checked_enum_application(ty) {
                let case = definition.resolve_case(*case)?;
                let payload = &definition.cases.get(case.index)?.payload;
                return Some(Signature {
                    params: payload
                        .iter()
                        .map(|field| {
                            let move_type = self.checked_payload_requires_move(&field.ty);
                            Parameter {
                                name: field.name.clone(),
                                move_type,
                                class_type: resolved_type_class(&field.ty).is_some(),
                                generic: false,
                                take: move_type,
                                writable: false,
                                borrow: false,
                            }
                        })
                        .collect(),
                    returns_move_type: !definition.capabilities.copy,
                    ..Signature::default()
                });
            }
        }
        let class = self.qualifier_class(qualifier)?;
        self.enum_cases
            .get(&(class.clone(), method.to_string()))
            .or_else(|| self.methods.get(&(class, method.to_string())))
            .cloned()
    }

    fn checked_enum_application(
        &self,
        ty: &crate::enums::EnumType,
    ) -> Option<&crate::semantics::EnumSemanticInfo> {
        self.enum_semantics
            .iter()
            .find(|definition| definition.source_type == *ty)
    }

    fn checked_payload_requires_move(&self, ty: &ResolvedType) -> bool {
        match ty {
            ResolvedType::Enum(ty) => self.checked_enum_application(ty).map_or_else(
                || !ty.arguments.is_empty() || self.move_enum_names.declaration_is_move(&ty.name),
                |definition| !definition.capabilities.copy,
            ),
            ResolvedType::Nullable(inner) => self.checked_payload_requires_move(inner),
            _ => {
                resolved_type_is_move_type(ty, &self.move_enum_names)
                    || resolved_type_requires_conservative_move(ty)
            }
        }
    }

    fn resolved_type(&self, expr: &Expr) -> Option<&crate::types::ResolvedType> {
        self.resolved_types.get(&expr.span())
    }

    fn call_arg_mode(&self, signature: &Signature, index: usize, arg: &Expr) -> UseMode {
        let Some(param) = signature.params.get(index) else {
            return UseMode::Read;
        };
        let move_type = param.move_type
            || (param.generic
                && self.resolved_type(arg).is_some_and(|ty| {
                    resolved_type_is_move_type(ty, &self.move_enum_names)
                        || matches!(ty, crate::types::ResolvedType::TypeParameter(_))
                }));
        if param.take && move_type {
            UseMode::Give
        } else if param.writable && move_type {
            UseMode::Write
        } else {
            UseMode::Read
        }
    }
}

fn variable_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Variable { name, .. } => Some(name),
        Expr::Grouped { expr, .. } => variable_name(expr),
        _ => None,
    }
}

fn borrow_modes_conflict(existing: UseMode, requested: UseMode) -> bool {
    matches!(
        (existing, requested),
        (UseMode::Write, UseMode::Read)
            | (UseMode::Read, UseMode::Write)
            | (UseMode::Write, UseMode::Write)
    )
}

fn binding_identity_key(id: OwnershipSlotId, name: &str) -> String {
    format!("binding:{}:{name}", id.0)
}

fn binding_roots(binding: &Binding, name: &str) -> Vec<BorrowPlace> {
    binding
        .borrow_root
        .as_ref()
        .map(|source| source.places().to_vec())
        .unwrap_or_else(|| vec![binding_identity_key(binding.id, name).into()])
}

fn display_borrow_root(place: &BorrowPlace) -> String {
    let root = place.owner.as_str();
    if root == "$this" {
        root.to_string()
    } else if let Some(member) = root.strip_prefix("static:") {
        member.to_string()
    } else if let Some(binding) = root
        .strip_prefix("binding:")
        .and_then(|root| root.split_once(':').map(|(_, binding)| binding))
    {
        format!("${binding}")
    } else {
        format!("${root}")
    }
}

fn statements_use_variable(statements: &[Stmt], name: &str) -> bool {
    for statement in statements {
        if statement_uses_variable(statement, name) {
            return true;
        }
        if matches!(
            statement,
            Stmt::VarDecl(declaration)
                if declaration
                    .bindings
                    .iter()
                    .any(|binding| binding.name == name)
        ) {
            return false;
        }
    }
    false
}

fn statement_uses_variable(statement: &Stmt, name: &str) -> bool {
    match statement {
        Stmt::Block(block) => statements_use_variable(&block.statements, name),
        Stmt::VarDecl(declaration) => expr_uses_variable(&declaration.initializer, name),
        Stmt::Assignment(assignment) => {
            expr_uses_variable(&assignment.target, name)
                || expr_uses_variable(&assignment.value, name)
        }
        Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => expr_uses_variable(expr, name),
        Stmt::Return { expr, .. } => expr
            .as_ref()
            .is_some_and(|expr| expr_uses_variable(expr, name)),
        Stmt::If(statement) => {
            statement
                .given
                .as_ref()
                .is_some_and(|given| statements_use_variable(&given.block.statements, name))
                || expr_uses_variable(&statement.condition, name)
                || statements_use_variable(&statement.then_block.statements, name)
                || statement
                    .else_branch
                    .as_ref()
                    .is_some_and(|branch| match branch {
                        ast::ElseBranch::If(statement) => {
                            statement_uses_variable(&Stmt::If((**statement).clone()), name)
                        }
                        ast::ElseBranch::Block(block) => {
                            statements_use_variable(&block.statements, name)
                        }
                    })
                || statement
                    .finally
                    .as_ref()
                    .is_some_and(|finally| statements_use_variable(&finally.block.statements, name))
        }
        Stmt::While(statement) => {
            statement
                .given
                .as_ref()
                .is_some_and(|given| statements_use_variable(&given.block.statements, name))
                || expr_uses_variable(&statement.condition, name)
                || statements_use_variable(&statement.body.statements, name)
                || statement
                    .finally
                    .as_ref()
                    .is_some_and(|finally| statements_use_variable(&finally.block.statements, name))
        }
        Stmt::DoWhile(statement) => {
            statements_use_variable(&statement.body.statements, name)
                || expr_uses_variable(&statement.condition, name)
                || statement
                    .finally
                    .as_ref()
                    .is_some_and(|finally| statements_use_variable(&finally.block.statements, name))
        }
        Stmt::For(statement) => {
            let initializer_uses = statement
                .initializer
                .as_ref()
                .is_some_and(|initializer| for_initializer_uses_variable(initializer, name));
            let initializer_shadows = matches!(
                statement.initializer.as_ref(),
                Some(ast::ForInitializer::VarDecl(declaration))
                    if declaration
                        .bindings
                        .iter()
                        .any(|binding| binding.name == name)
            );
            initializer_uses
                || (!initializer_shadows
                    && (statement
                        .condition
                        .as_ref()
                        .is_some_and(|expr| expr_uses_variable(expr, name))
                        || statement
                            .increment
                            .as_ref()
                            .is_some_and(|increment| for_increment_uses_variable(increment, name))
                        || statements_use_variable(&statement.body.statements, name)))
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => false,
        Stmt::Foreach(statement) => {
            expr_uses_variable(&statement.iterable, name)
                || (statement
                    .first_binding
                    .as_ref()
                    .is_none_or(|first_binding| first_binding.name != name)
                    && statement.value_binding.name != name
                    && statements_use_variable(&statement.body.statements, name))
        }
        Stmt::Increment(statement) => expr_uses_variable(&statement.target, name),
        Stmt::Throw(statement) => expr_uses_variable(&statement.expr, name),
        Stmt::Try(statement) => {
            statements_use_variable(&statement.body.statements, name)
                || statement.catches.iter().any(|catch| {
                    catch
                        .binding
                        .as_ref()
                        .is_none_or(|binding| binding.name != name)
                        && statements_use_variable(&catch.body.statements, name)
                })
                || statement
                    .finally
                    .as_ref()
                    .is_some_and(|finally| statements_use_variable(&finally.body.statements, name))
        }
    }
}

fn for_initializer_uses_variable(initializer: &ast::ForInitializer, name: &str) -> bool {
    match initializer {
        ast::ForInitializer::VarDecl(declaration) => {
            expr_uses_variable(&declaration.initializer, name)
        }
        ast::ForInitializer::Assignment(assignment) => {
            expr_uses_variable(&assignment.target, name)
                || expr_uses_variable(&assignment.value, name)
        }
    }
}

fn for_increment_uses_variable(increment: &ast::ForIncrement, name: &str) -> bool {
    match increment {
        ast::ForIncrement::Increment(increment) => expr_uses_variable(&increment.target, name),
        ast::ForIncrement::Assignment(assignment) => {
            expr_uses_variable(&assignment.target, name)
                || expr_uses_variable(&assignment.value, name)
        }
    }
}

fn expr_uses_variable(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Variable {
            name: candidate, ..
        } => candidate == name,
        Expr::InterpolatedString { parts, .. } => parts.iter().any(|part| match part {
            ast::InterpolatedStringPart::Text { .. } => false,
            ast::InterpolatedStringPart::Expr(expr) => expr_uses_variable(expr, name),
        }),
        Expr::Array { elements, .. } => elements.iter().any(|element| {
            element
                .key
                .as_ref()
                .is_some_and(|key| expr_uses_variable(key, name))
                || expr_uses_variable(&element.value, name)
        }),
        Expr::ArrayRepeat { value, count, .. } => {
            expr_uses_variable(value, name) || expr_uses_variable(count, name)
        }
        Expr::Index {
            collection, index, ..
        } => expr_uses_variable(collection, name) || expr_uses_variable(index, name),
        Expr::PropertyAccess { object, .. }
        | Expr::MethodCall { object, .. }
        | Expr::IsType { expr: object, .. }
        | Expr::Grouped { expr: object, .. }
        | Expr::Unary { expr: object, .. } => {
            expr_uses_variable(object, name)
                || matches!(expr, Expr::MethodCall { args, .. } if arguments_use_variable(args, name))
        }
        Expr::FunctionCall { args, .. }
        | Expr::StaticCall { args, .. }
        | Expr::New { args, .. } => arguments_use_variable(args, name),
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
            ..
        } => expr_uses_variable(left, name) || expr_uses_variable(right, name),
        Expr::Match {
            scrutinee, arms, ..
        } => {
            expr_uses_variable(scrutinee, name)
                || arms.iter().any(|arm| {
                    let pattern_uses = match &arm.pattern {
                        ast::MatchPattern::Expression(pattern) => expr_uses_variable(pattern, name),
                        ast::MatchPattern::Default { .. }
                        | ast::MatchPattern::EnumCase { .. }
                        | ast::MatchPattern::TypeBinding { .. } => false,
                    };
                    pattern_uses
                        || (!match_pattern_binds(&arm.pattern, name)
                            && expr_uses_variable(&arm.value, name))
                })
        }
        Expr::When(when) => {
            when.given
                .as_ref()
                .is_some_and(|given| statements_use_variable(&given.block.statements, name))
                || when.branches.iter().any(|branch| {
                    branch
                        .condition
                        .as_ref()
                        .is_some_and(|condition| expr_uses_variable(condition, name))
                        || statements_use_variable(&branch.block.statements, name)
                })
                || when
                    .finally
                    .as_ref()
                    .is_some_and(|finally| statements_use_variable(&finally.block.statements, name))
        }
        Expr::Closure(closure) => {
            closure
                .captures
                .as_ref()
                .is_some_and(|clause| clause.captures.iter().any(|capture| capture.name == name))
                || match &closure.body {
                    ast::ClosureBody::Expression { expression, .. } => {
                        expr_uses_variable(expression, name)
                    }
                    ast::ClosureBody::Block(block) => {
                        statements_use_variable(&block.statements, name)
                    }
                }
        }
        Expr::CallableCall { callee, args, .. } => {
            expr_uses_variable(callee, name) || arguments_use_variable(args, name)
        }
        Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::StaticMember { .. } => false,
    }
}

fn match_pattern_bindings(pattern: &ast::MatchPattern) -> Vec<&ast::MatchBinding> {
    match pattern {
        ast::MatchPattern::EnumCase {
            bindings: Some(bindings),
            ..
        } => bindings.iter().collect(),
        ast::MatchPattern::TypeBinding { binding, .. } => vec![binding],
        ast::MatchPattern::Default { .. }
        | ast::MatchPattern::Expression(_)
        | ast::MatchPattern::EnumCase { bindings: None, .. } => Vec::new(),
    }
}

fn match_pattern_binds(pattern: &ast::MatchPattern, name: &str) -> bool {
    match_pattern_bindings(pattern)
        .into_iter()
        .any(|binding| binding.name == name)
}

fn arguments_use_variable(arguments: &[Argument], name: &str) -> bool {
    arguments
        .iter()
        .any(|argument| expr_uses_variable(&argument.value, name))
}

fn type_ref_class_name(
    ty: &crate::types::TypeRef,
    classes: &HashSet<String>,
    receiver_class: Option<&str>,
) -> Option<String> {
    if ty.name == "self" {
        let owner = receiver_class?;
        return crate::inline_value::InlineValueKind::for_name(owner)
            .is_none()
            .then(|| owner.to_string());
    }
    (classes.contains(&ty.name)
        && crate::inline_value::InlineValueKind::for_name(&ty.name).is_none())
    .then(|| ty.name.clone())
}

fn resolved_type_class(ty: &crate::types::ResolvedType) -> Option<&str> {
    match ty {
        crate::types::ResolvedType::Class(class) if class.inline_copy_type().is_none() => {
            Some(&class.name)
        }
        crate::types::ResolvedType::Nullable(inner) => resolved_type_class(inner),
        crate::types::ResolvedType::SharedHandle(kind, payload) if kind.forwards_payload() => {
            resolved_type_class(payload)
        }
        _ => None,
    }
}

/// Which enum applications own their values (Decision 0114). A concrete
/// application uses its own instance's capability; a symbolic application in
/// a generic body uses its declaration's conservative template capability.
#[derive(Debug, Clone, Default)]
pub struct MoveEnums {
    declarations: HashSet<String>,
    applications: HashMap<crate::enums::EnumType, bool>,
}

impl MoveEnums {
    pub fn new(
        declarations: impl IntoIterator<Item = String>,
        applications: impl IntoIterator<Item = (crate::enums::EnumType, bool)>,
    ) -> Self {
        Self {
            declarations: declarations.into_iter().collect(),
            applications: applications.into_iter().collect(),
        }
    }

    pub fn is_move(&self, ty: &crate::enums::EnumType) -> bool {
        self.applications
            .get(ty)
            .copied()
            .unwrap_or_else(|| self.declarations.contains(&ty.name))
    }

    pub fn declaration_is_move(&self, name: &str) -> bool {
        self.declarations.contains(name)
    }

    pub fn declarations(&self) -> impl Iterator<Item = &String> {
        self.declarations.iter()
    }
}

pub(crate) fn resolved_type_is_move_type(
    ty: &crate::types::ResolvedType,
    move_enum_names: &MoveEnums,
) -> bool {
    match ty {
        crate::types::ResolvedType::ByteRegion(kind) => kind.is_writable(),
        crate::types::ResolvedType::Class(class) => class.inline_copy_type().is_none(),
        crate::types::ResolvedType::Bytes
        | crate::types::ResolvedType::Mixed
        | crate::types::ResolvedType::Error
        | crate::types::ResolvedType::Function(_)
        | crate::types::ResolvedType::Interface(_)
        | crate::types::ResolvedType::SharedHandle(_, _)
        | crate::types::ResolvedType::TypedArray(_)
        | crate::types::ResolvedType::List(_)
        | crate::types::ResolvedType::Dictionary(_, _)
        | crate::types::ResolvedType::SortedDictionary(_, _)
        | crate::types::ResolvedType::Set(_)
        | crate::types::ResolvedType::SortedSet(_)
        | crate::types::ResolvedType::PriorityQueue(_)
        | crate::types::ResolvedType::Deque(_) => true,
        crate::types::ResolvedType::Enum(enum_type) => move_enum_names.is_move(enum_type),
        crate::types::ResolvedType::Nullable(inner) => {
            resolved_type_is_move_type(inner, move_enum_names)
        }
        _ => false,
    }
}

fn resolved_shared_handle_kind(
    ty: &crate::types::ResolvedType,
) -> Option<crate::types::SharedHandleKind> {
    match ty {
        crate::types::ResolvedType::SharedHandle(kind, _) => Some(*kind),
        crate::types::ResolvedType::Nullable(inner) => resolved_shared_handle_kind(inner),
        _ => None,
    }
}

pub(crate) fn resolved_type_requires_conservative_move(ty: &crate::types::ResolvedType) -> bool {
    match ty {
        crate::types::ResolvedType::TypeParameter(_)
        | crate::types::ResolvedType::TraitSelf(_)
        | crate::types::ResolvedType::InterfaceSelf(_)
        | crate::types::ResolvedType::Unsupported => true,
        crate::types::ResolvedType::Nullable(inner)
        | crate::types::ResolvedType::TypedArray(inner)
        | crate::types::ResolvedType::List(inner)
        | crate::types::ResolvedType::Set(inner)
        | crate::types::ResolvedType::SortedSet(inner)
        | crate::types::ResolvedType::PriorityQueue(inner)
        | crate::types::ResolvedType::Deque(inner) => {
            resolved_type_requires_conservative_move(inner)
        }
        crate::types::ResolvedType::Dictionary(key, value)
        | crate::types::ResolvedType::SortedDictionary(key, value) => {
            resolved_type_requires_conservative_move(key)
                || resolved_type_requires_conservative_move(value)
        }
        crate::types::ResolvedType::Class(class) => {
            class.inline_copy_type().is_none()
                && class
                    .arguments
                    .iter()
                    .any(resolved_type_requires_conservative_move)
        }
        crate::types::ResolvedType::SharedHandle(_, payload) => {
            resolved_type_requires_conservative_move(payload)
        }
        _ => false,
    }
}

fn resolved_collection_info(
    ty: &crate::types::ResolvedType,
    move_enum_names: &MoveEnums,
) -> Option<CollectionInfo> {
    use crate::types::ResolvedType;
    let (family, value) = match ty {
        ResolvedType::Bytes => {
            return Some(CollectionInfo {
                family: CollectionFamily::Bytes,
                value_move: false,
                value_mixed: false,
                value_class: None,
                value_collection: None,
            });
        }
        ResolvedType::TypedArray(value) => (CollectionFamily::TypedArray, value.as_ref()),
        ResolvedType::List(value) => (CollectionFamily::List, value.as_ref()),
        ResolvedType::Dictionary(_, value) | ResolvedType::SortedDictionary(_, value) => {
            (CollectionFamily::Dictionary, value.as_ref())
        }
        ResolvedType::Set(value) | ResolvedType::SortedSet(value) => {
            (CollectionFamily::Set, value.as_ref())
        }
        ResolvedType::PriorityQueue(value) => (CollectionFamily::PriorityQueue, value.as_ref()),
        ResolvedType::Deque(value) => (CollectionFamily::Deque, value.as_ref()),
        ResolvedType::Nullable(inner) => {
            return resolved_collection_info(inner, move_enum_names);
        }
        _ => return None,
    };
    Some(CollectionInfo {
        family,
        value_move: resolved_type_is_move_type(value, move_enum_names)
            || resolved_type_requires_conservative_move(value),
        value_mixed: resolved_type_is_mixed(value),
        value_class: resolved_type_class(value).map(str::to_string),
        value_collection: resolved_collection_info(value, move_enum_names).map(Box::new),
    })
}

fn resolved_type_is_mixed(ty: &crate::types::ResolvedType) -> bool {
    match ty {
        crate::types::ResolvedType::Mixed => true,
        crate::types::ResolvedType::Nullable(inner) => resolved_type_is_mixed(inner),
        _ => false,
    }
}

fn type_ref_collection_info(
    ty: &crate::types::TypeRef,
    classes: &HashSet<String>,
    move_nominal_names: &HashSet<String>,
    receiver_class: Option<&str>,
    type_params: &[ast::TypeParamDecl],
    enclosing_type_params: &[ast::TypeParamDecl],
) -> Option<CollectionInfo> {
    if ty.name == "Bytes" && ty.arguments.is_empty() {
        return Some(bytes_collection_info());
    }
    let (family, value) = match ty.name.as_str() {
        "[]" if ty.type_argument_count() == 1 => {
            (CollectionFamily::TypedArray, ty.type_argument(0)?)
        }
        "List" if ty.type_argument_count() == 1 => (CollectionFamily::List, ty.type_argument(0)?),
        "Dictionary" if ty.type_argument_count() == 2 => {
            (CollectionFamily::Dictionary, ty.type_argument(1)?)
        }
        "SortedDictionary" if ty.type_argument_count() == 2 => {
            (CollectionFamily::Dictionary, ty.type_argument(1)?)
        }
        "Set" if ty.type_argument_count() == 1 => (CollectionFamily::Set, ty.type_argument(0)?),
        "SortedSet" if ty.type_argument_count() == 1 => {
            (CollectionFamily::Set, ty.type_argument(0)?)
        }
        "PriorityQueue" if ty.type_argument_count() == 1 => {
            (CollectionFamily::PriorityQueue, ty.type_argument(0)?)
        }
        "Deque" if ty.type_argument_count() == 1 => (CollectionFamily::Deque, ty.type_argument(0)?),
        _ => return None,
    };
    Some(CollectionInfo {
        family,
        value_move: type_ref_is_move_type_with_nominals(
            value,
            classes,
            move_nominal_names,
            receiver_class,
        ) || type_ref_has_any_ownership_parameter(
            value,
            type_params,
            enclosing_type_params,
        ),
        value_mixed: value.name == "mixed",
        value_class: type_ref_class_name(value, classes, receiver_class),
        value_collection: type_ref_collection_info(
            value,
            classes,
            move_nominal_names,
            receiver_class,
            type_params,
            enclosing_type_params,
        )
        .map(Box::new),
    })
}

fn bytes_collection_info() -> CollectionInfo {
    CollectionInfo {
        family: CollectionFamily::Bytes,
        value_move: false,
        value_mixed: false,
        value_class: None,
        value_collection: None,
    }
}

fn byte_array_collection_info() -> CollectionInfo {
    CollectionInfo {
        family: CollectionFamily::TypedArray,
        value_move: false,
        value_mixed: false,
        value_class: None,
        value_collection: None,
    }
}

pub(crate) fn type_ref_is_move_type(
    ty: &crate::types::TypeRef,
    classes: &HashSet<String>,
    receiver_class: Option<&str>,
) -> bool {
    type_ref_is_move_type_with_nominals(ty, classes, &HashSet::new(), receiver_class)
}

fn type_ref_is_move_type_with_nominals(
    ty: &crate::types::TypeRef,
    classes: &HashSet<String>,
    move_nominal_names: &HashSet<String>,
    receiver_class: Option<&str>,
) -> bool {
    if crate::inline_value::InlineValueKind::for_nominal(&ty.name, ty.arguments.len()).is_some() {
        return false;
    }
    // Every Stage 25a handle and access object is a move type (record 0106):
    // plain assignment transfers the handle and never silently retains.
    ty.function.is_some()
        || crate::types::ByteRegionKind::from_source_name(&ty.name)
            .is_some_and(crate::types::ByteRegionKind::is_writable)
        || crate::types::SharedHandleKind::from_source_name(&ty.name).is_some()
        || type_ref_class_name(ty, classes, receiver_class).is_some()
        || move_nominal_names.contains(&ty.name)
        || matches!(
            ty.name.as_str(),
            "mixed"
                | "Error"
                | "Bytes"
                | "[]"
                | "List"
                | "Dictionary"
                | "SortedDictionary"
                | "Set"
                | "SortedSet"
                | "PriorityQueue"
                | "Deque"
        )
}

fn non_null_function_type(
    ty: &ResolvedType,
) -> Option<&crate::types::SemanticFunctionType<ResolvedType>> {
    match ty {
        ResolvedType::Function(function) => Some(function),
        ResolvedType::Nullable(inner) => non_null_function_type(inner),
        _ => None,
    }
}

pub(crate) fn constant_bool(expr: &Expr) -> Option<bool> {
    match expr {
        Expr::Bool { value, .. } => Some(*value),
        Expr::Grouped { expr, .. } => constant_bool(expr),
        Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
            ..
        } => constant_bool(expr).map(|value| !value),
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
            ..
        } => match constant_bool(left) {
            Some(false) => Some(false),
            Some(true) => constant_bool(right),
            None if constant_bool(right) == Some(false) => Some(false),
            None => None,
        },
        Expr::Binary {
            left,
            op: BinaryOp::Or,
            right,
            ..
        } => match constant_bool(left) {
            Some(true) => Some(true),
            Some(false) => constant_bool(right),
            None if constant_bool(right) == Some(true) => Some(true),
            None => None,
        },
        Expr::Binary {
            left,
            op: BinaryOp::Xor,
            right,
            ..
        } => Some(constant_bool(left)? ^ constant_bool(right)?),
        _ => None,
    }
}

fn is_panic_expr(expr: &Expr) -> bool {
    match expr {
        Expr::FunctionCall { name, .. } => name == "panic",
        Expr::Grouped { expr, .. } => is_panic_expr(expr),
        _ => false,
    }
}

fn merge_loop_exit(scopes: &mut Scopes, before: &Scopes, backedges: &[Scopes]) {
    let Some((first, rest)) = backedges.split_first() else {
        *scopes = before.clone();
        return;
    };
    let mut repeated = first.clone();
    for state in rest {
        let left = repeated.clone();
        repeated.merge_from(&left, state);
    }
    scopes.merge_from(before, &repeated);
}

fn merge_reachable_states(scopes: &mut Scopes, states: &[Scopes]) {
    let Some((first, rest)) = states.split_first() else {
        return;
    };
    let mut merged = first.clone();
    for state in rest {
        let left = merged.clone();
        merged.merge_from(&left, state);
    }
    *scopes = merged;
}

#[cfg(test)]
mod return_contract_tests {
    use super::*;

    #[test]
    fn function_value_signatures_preserve_symbolic_ownership_until_substitution() {
        use crate::types::{
            FunctionTypeParameterMode, SemanticFunctionParameter, SemanticFunctionType,
        };
        for (ty, symbolic, owned) in [
            (ResolvedType::TypeParameter("T".into()), true, true),
            (ResolvedType::TraitSelf("Factory".into()), true, true),
            (ResolvedType::InterfaceSelf("Factory".into()), true, true),
            (
                ResolvedType::Integer(crate::types::IntegerType::Int64),
                false,
                false,
            ),
            (
                ResolvedType::Class(crate::types::ClassType::new("Item", vec![])),
                false,
                true,
            ),
        ] {
            let mut function = SemanticFunctionType {
                invocation_mode: FunctionInvocationMode::Readonly,
                parameters: vec![SemanticFunctionParameter {
                    ownership_mode: FunctionTypeParameterMode::Take,
                    ty: ty.clone(),
                }],
                return_type: ty,
                checked_effects: vec![],
                return_borrow: None,
            };
            let signature = Signature::for_function_value(&function, None, &MoveEnums::default());
            assert_eq!(signature.params[0].generic, symbolic);
            assert_eq!(signature.returns_move_type, owned);
            function.return_borrow = Some(crate::types::FunctionReturnBorrow {
                kind: ReturnBorrowKind::Value,
                source: crate::types::FunctionBorrowSource::Parameter(0),
                writable: false,
                lifetime: RetainedLifetime::LastUse,
            });
            assert!(
                !Signature::for_function_value(&function, None, &MoveEnums::default())
                    .returns_move_type
            );
        }
    }

    fn contract(kind: ReturnBorrowKind, writable: bool) -> ReturnBorrow {
        ReturnBorrow {
            kind,
            source: BorrowSource::Parameter(0),
            writable,
            lifetime: RetainedLifetime::LastUse,
        }
    }

    #[test]
    fn aliases_join_output_capabilities_and_retained_demands_separately() {
        let alias = |kind, writable| {
            ReturnBorrowAlias::Places(BorrowOrigins {
                sources: vec![BorrowOrigin::Parameter(0)],
                kind,
                writable,
                lifetime: RetainedLifetime::LastUse,
                borrowed: true,
            })
        };
        for kind in [ReturnBorrowKind::Value, ReturnBorrowKind::Retained] {
            let readonly = alias(kind, false);
            let writable = alias(kind, true);
            let expected = alias(kind, kind == ReturnBorrowKind::Retained);
            assert_eq!(readonly.clone().join(writable.clone()), expected);
            assert_eq!(writable.clone().join(readonly), expected);
            assert_eq!(ReturnBorrowAlias::Null.join(writable.clone()), writable);
        }
        assert_eq!(
            alias(ReturnBorrowKind::Value, false).join(alias(ReturnBorrowKind::Retained, false)),
            ReturnBorrowAlias::Unknown
        );
    }

    #[test]
    fn local_and_alternative_origins_do_not_become_public_return_contracts() {
        let origins = BorrowOrigins {
            sources: vec![BorrowOrigin::Parameter(0)],
            kind: ReturnBorrowKind::Value,
            lifetime: RetainedLifetime::LastUse,
            writable: false,
            borrowed: true,
        };
        assert_eq!(
            origins.contract(),
            Some(contract(ReturnBorrowKind::Value, false))
        );
        for sources in [
            vec![BorrowOrigin::Local(BindingId(7))],
            vec![BorrowOrigin::Parameter(0), BorrowOrigin::Parameter(1)],
        ] {
            assert_eq!(
                BorrowOrigins {
                    sources,
                    ..origins.clone()
                }
                .contract(),
                None
            );
        }
        assert_eq!(
            BorrowOrigins {
                borrowed: false,
                ..origins
            }
            .contract(),
            None
        );
    }

    #[test]
    fn forwarding_preserves_retained_source_demands_without_upgrading_value_access() {
        let source = contract(ReturnBorrowKind::Value, false);
        assert!(!returned_source_access(
            source,
            contract(ReturnBorrowKind::Value, true)
        ));
        assert!(returned_source_access(
            source,
            contract(ReturnBorrowKind::Retained, true)
        ));
        assert!(returned_source_access(
            contract(ReturnBorrowKind::Retained, true),
            contract(ReturnBorrowKind::Retained, false)
        ));
    }
}
