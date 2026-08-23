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
use crate::types::{FunctionInvocationMode, ResolvedType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowAccess {
    Readonly,
    Writable,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClosureBorrowRoot {
    Binding(BindingId),
    Receiver,
    EnclosingEnvironment(ClosureId),
    Temporary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosureValueProvenance {
    Owned,
    BorrowBound(Vec<ClosureBorrowRoot>),
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
    pub roots: Vec<ClosureBorrowRoot>,
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
    pub provenance: ClosureValueProvenance,
    pub acquisitions: Vec<CaptureAcquisition>,
    pub release_order: Vec<usize>,
    pub escape: ClosureEscapeClassification,
    pub invocation_consumption: InvocationConsumption,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OwnershipAnalysis {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) closures: HashMap<ClosureId, ClosureOwnershipInfo>,
}

#[derive(Debug, Clone)]
struct Parameter {
    name: String,
    move_type: bool,
    class_type: bool,
    generic: bool,
    take: bool,
    writable: bool,
}

#[derive(Debug, Clone, Default)]
struct Signature {
    params: Vec<Parameter>,
    returns: Option<String>,
    returns_collection: Option<CollectionInfo>,
    returns_move_type: bool,
    return_borrow: Option<ReturnBorrow>,
    receiver: Option<UseMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReturnedClosureProvenance {
    Owned,
    Borrowed(ReturnBorrow),
    Invalid,
}

impl Signature {
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
        }],
        returns_move_type: true,
        ..Signature::default()
    }
}

fn returned_closure_provenance(
    function: &ast::FunctionDecl,
    closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    binding_resolution: &BindingResolution,
) -> Option<ReturnedClosureProvenance> {
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
            (ReturnedClosureProvenance::Owned, ReturnedClosureProvenance::Borrowed(borrow))
            | (ReturnedClosureProvenance::Borrowed(borrow), ReturnedClosureProvenance::Owned) => {
                ReturnedClosureProvenance::Borrowed(borrow)
            }
            (
                ReturnedClosureProvenance::Borrowed(current),
                ReturnedClosureProvenance::Borrowed(incoming),
            ) if current.source == incoming.source => {
                ReturnedClosureProvenance::Borrowed(ReturnBorrow {
                    source: current.source,
                    writable: current.writable && incoming.writable,
                })
            }
            (ReturnedClosureProvenance::Borrowed(_), ReturnedClosureProvenance::Borrowed(_)) => {
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
            if let Some(id) = binding_resolution
                .declaration_by_span
                .get(&(binding.span.start, binding.span.end))
            {
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
        if let Some(id) = binding_resolution.uses_by_span.get(&(span.start, span.end)) {
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
        sources: &HashMap<BindingId, Vec<Expr>>,
        visiting: &mut HashSet<BindingId>,
    ) -> Option<ReturnedClosureProvenance> {
        match ungroup_expr(expr) {
            Expr::Closure(closure) => {
                let semantic = closures.get(&ClosureId::from_span(closure.span))?;
                let mut borrow: Option<ReturnBorrow> = None;
                for capture in &semantic.captures {
                    if capture.mode == ast::ClosureCaptureMode::Take {
                        continue;
                    }
                    let declaration = binding_resolution
                        .declarations_by_id
                        .get(&capture.source_binding_id)?;
                    let candidate = match declaration.kind {
                        BindingKind::MethodReceiver => ReturnBorrow {
                            source: BorrowSource::Receiver,
                            writable: capture.mode == ast::ClosureCaptureMode::Writable,
                        },
                        BindingKind::FunctionParameter | BindingKind::MethodParameter => {
                            let index = function
                                .params
                                .iter()
                                .position(|parameter| parameter.name == declaration.name)?;
                            ReturnBorrow {
                                source: BorrowSource::Parameter(index),
                                writable: capture.mode == ast::ClosureCaptureMode::Writable,
                            }
                        }
                        _ => return Some(ReturnedClosureProvenance::Invalid),
                    };
                    match borrow {
                        Some(existing) if existing.source != candidate.source => {
                            return Some(ReturnedClosureProvenance::Invalid);
                        }
                        Some(existing) => {
                            borrow = Some(ReturnBorrow {
                                writable: existing.writable && candidate.writable,
                                ..existing
                            });
                        }
                        None => borrow = Some(candidate),
                    }
                }
                Some(borrow.map_or(
                    ReturnedClosureProvenance::Owned,
                    ReturnedClosureProvenance::Borrowed,
                ))
            }
            Expr::Variable { span, .. } => {
                let id = *binding_resolution
                    .uses_by_span
                    .get(&(span.start, span.end))?;
                if !visiting.insert(id) {
                    return Some(ReturnedClosureProvenance::Invalid);
                }
                let mut found = None;
                for source in sources.get(&id)? {
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
                visiting.remove(&id);
                found
            }
            _ => None,
        }
    }

    fn visit_block(
        block: &ast::Block,
        function: &ast::FunctionDecl,
        closures: &HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
        binding_resolution: &BindingResolution,
        sources: &HashMap<BindingId, Vec<Expr>>,
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
    collect_sources(&function.body, binding_resolution, &mut sources);
    let mut found = None;
    visit_block(
        &function.body,
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
    Owned(&'a str),
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
    borrow_root: Option<String>,
    writable: bool,
    state: State,
    function_type: Option<crate::types::SemanticFunctionType<ResolvedType>>,
    function_value: Option<FunctionValueState>,
    scope_depth: usize,
}

#[derive(Debug, Clone)]
struct FunctionValueState {
    closure_id: Option<ClosureId>,
    provenance: ClosureValueProvenance,
    leases: Vec<ClosureLease>,
    nonescaping_parameter: bool,
    take_parameter_insertion: Option<Span>,
}

#[derive(Debug, Clone)]
struct ClosureLease {
    root: ClosureBorrowRoot,
    root_key: String,
    access: BorrowAccess,
    capture_span: Span,
    source_depth: usize,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollectionFamily {
    TypedArray,
    List,
    Dictionary,
    Set,
    PriorityQueue,
    Deque,
    Bytes,
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
struct Scopes(Vec<HashMap<String, Option<Binding>>>);

impl Scopes {
    fn new() -> Self {
        Self(vec![HashMap::new()])
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

    fn borrowed_from(&self, root: &str) -> Option<&str> {
        self.0.iter().rev().find_map(|scope| {
            scope.iter().find_map(|(name, binding)| {
                binding
                    .as_ref()
                    .is_some_and(|binding| binding.borrow_root.as_deref() == Some(root))
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

    fn active_closure_leases(&self) -> impl Iterator<Item = &ClosureLease> {
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
                        .function_value
                        .as_ref()
                        .filter(move |_| available)
                        .into_iter()
                        .flat_map(|value| value.leases.iter())
                })
        })
    }

    fn release_unused_closure_leases(&mut self, remaining: &[Stmt], current_scope_only: bool) {
        let start = if current_scope_only {
            self.0.len().saturating_sub(1)
        } else {
            0
        };
        for scope in &mut self.0[start..] {
            for (name, binding) in scope {
                let Some(binding) = binding else {
                    continue;
                };
                if binding.function_value.is_some() && !statements_use_variable(remaining, name) {
                    if let Some(value) = &mut binding.function_value {
                        value.leases.clear();
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
                binding.function_value = join_function_value(
                    left_state.function_value.as_ref(),
                    right_state.function_value.as_ref(),
                );
            }
        }
    }
}

fn join_function_value(
    left: Option<&FunctionValueState>,
    right: Option<&FunctionValueState>,
) -> Option<FunctionValueState> {
    match (left, right) {
        (Some(left), Some(right)) if left.closure_id == right.closure_id => {
            let mut joined = left.clone();
            for lease in &right.leases {
                if !joined.leases.iter().any(|candidate| {
                    candidate.root == lease.root && candidate.access == lease.access
                }) {
                    joined.leases.push(lease.clone());
                }
            }
            Some(joined)
        }
        (Some(left), Some(right)) => {
            let mut joined = left.clone();
            joined.closure_id = None;
            for lease in &right.leases {
                if !joined.leases.iter().any(|candidate| {
                    candidate.root == lease.root && candidate.access == lease.access
                }) {
                    joined.leases.push(lease.clone());
                }
            }
            let mut roots = provenance_roots(&left.provenance);
            roots.extend(provenance_roots(&right.provenance));
            roots.sort();
            roots.dedup();
            joined.provenance = if roots.is_empty() {
                ClosureValueProvenance::Owned
            } else {
                ClosureValueProvenance::BorrowBound(roots)
            };
            joined.nonescaping_parameter =
                left.nonescaping_parameter || right.nonescaping_parameter;
            Some(joined)
        }
        (Some(left), None) | (None, Some(left)) => Some(left.clone()),
        (None, None) => None,
    }
}

fn provenance_roots(provenance: &ClosureValueProvenance) -> Vec<ClosureBorrowRoot> {
    match provenance {
        ClosureValueProvenance::Owned => Vec::new(),
        ClosureValueProvenance::BorrowBound(roots) => roots.clone(),
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
    let move_enum_names = HashSet::new();
    let given_preludes = HashMap::new();
    let checked_effect_sites = HashMap::new();
    let catch_error_types = HashMap::new();
    let binding_resolution = BindingResolution::default();
    let closures = HashMap::new();
    let callable_value_calls = HashMap::new();
    check_program_with_inferred_move_returns(
        program,
        &OwnershipAnalysisContext {
            inferred_move_returns: &inferred_move_returns,
            return_borrows: &return_borrows,
            resolved_types: &resolved_types,
            flow_facts: &flow_facts,
            move_enum_names: &move_enum_names,
            given_preludes: &given_preludes,
            checked_effect_sites: &checked_effect_sites,
            catch_error_types: &catch_error_types,
            binding_resolution: &binding_resolution,
            closures: &closures,
            callable_value_calls: &callable_value_calls,
        },
    )
    .diagnostics
}

pub(crate) struct OwnershipAnalysisContext<'a> {
    pub(crate) inferred_move_returns: &'a HashSet<usize>,
    pub(crate) return_borrows: &'a HashMap<usize, ReturnBorrow>,
    pub(crate) resolved_types: &'a HashMap<(usize, usize), crate::types::ResolvedType>,
    pub(crate) flow_facts: &'a FactsByUse,
    pub(crate) move_enum_names: &'a HashSet<String>,
    pub(crate) given_preludes: &'a HashMap<(usize, usize), crate::semantics::GivenSemanticInfo>,
    pub(crate) checked_effect_sites: &'a crate::checked_effects::EffectSiteMap,
    pub(crate) catch_error_types: &'a crate::checked_effects::CatchTypeMap,
    pub(crate) binding_resolution: &'a BindingResolution,
    pub(crate) closures: &'a HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    pub(crate) callable_value_calls:
        &'a HashMap<(usize, usize), crate::semantics::CallableValueCallInfo>,
}

pub(crate) fn check_program_with_inferred_move_returns(
    program: &ast::Program,
    context: &OwnershipAnalysisContext<'_>,
) -> OwnershipAnalysis {
    let OwnershipAnalysisContext {
        inferred_move_returns,
        return_borrows,
        resolved_types,
        flow_facts,
        move_enum_names,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        binding_resolution,
        closures,
        callable_value_calls,
    } = *context;
    let classes = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class.name.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let class_type_params = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some((class.name.clone(), class.type_params.clone())),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut signatures = HashMap::new();
    let mut constructors = HashMap::new();
    let mut methods = HashMap::new();
    let mut enum_cases = HashMap::new();
    let mut properties = HashMap::new();
    let mut static_properties = HashMap::new();

    for item in &program.items {
        match item {
            Item::Function(function) => {
                let mut function_signature = signature(
                    function,
                    &classes,
                    move_enum_names,
                    inferred_move_returns,
                    return_borrows,
                    None,
                    &[],
                );
                if let Some(ReturnedClosureProvenance::Borrowed(borrow)) =
                    returned_closure_provenance(function, closures, binding_resolution)
                {
                    function_signature.return_borrow = Some(borrow);
                }
                signatures.insert(function.name.clone(), function_signature);
            }
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Property(property) if property.is_static => {
                            static_properties.insert(
                                (class.name.clone(), property.name.clone()),
                                property.writable,
                            );
                        }
                        ClassMember::Property(property) => {
                            let property_class =
                                type_ref_class_name(&property.ty, &classes, Some(&class.name));
                            let move_type =
                                type_ref_is_move_type_with_enums(
                                    &property.ty,
                                    &classes,
                                    move_enum_names,
                                    Some(&class.name),
                                ) || type_ref_mentions_parameter(&property.ty, &class.type_params);
                            properties.insert(
                                (class.name.clone(), property.name.clone()),
                                PropertyInfo {
                                    class: property_class,
                                    collection: type_ref_collection_info(
                                        &property.ty,
                                        &classes,
                                        move_enum_names,
                                        Some(&class.name),
                                        &class.type_params,
                                        &[],
                                    ),
                                    mixed: property.ty.name == "mixed",
                                    move_type,
                                    writable: property.writable,
                                },
                            );
                        }
                        ClassMember::Constant(_) => {}
                        ClassMember::Method(method) => {
                            let mut method_signature = signature(
                                method,
                                &classes,
                                move_enum_names,
                                inferred_move_returns,
                                return_borrows,
                                Some(&class.name),
                                &class.type_params,
                            );
                            if let Some(ReturnedClosureProvenance::Borrowed(borrow)) =
                                returned_closure_provenance(method, closures, binding_resolution)
                            {
                                method_signature.return_borrow = Some(borrow);
                            }
                            methods.insert(
                                (class.name.clone(), method.name.clone()),
                                method_signature.clone(),
                            );
                            if method.name == "__construct" {
                                constructors.insert(class.name.clone(), method_signature);
                                for param in &method.params {
                                    let property_class =
                                        type_ref_class_name(&param.ty, &classes, Some(&class.name));
                                    let move_type = type_ref_is_move_type_with_enums(
                                        &param.ty,
                                        &classes,
                                        move_enum_names,
                                        Some(&class.name),
                                    ) || type_ref_mentions_parameter(
                                        &param.ty,
                                        &class.type_params,
                                    );
                                    if param.promoted_access.is_some() {
                                        properties.insert(
                                            (class.name.clone(), param.name.clone()),
                                            PropertyInfo {
                                                class: property_class,
                                                collection: type_ref_collection_info(
                                                    &param.ty,
                                                    &classes,
                                                    move_enum_names,
                                                    Some(&class.name),
                                                    &class.type_params,
                                                    &[],
                                                ),
                                                mixed: param.ty.name == "mixed",
                                                move_type,
                                                writable: param.writable,
                                            },
                                        );
                                    }
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
                                    let generic = type_ref_mentions_parameter(
                                        &field.ty,
                                        &declaration.type_params,
                                    );
                                    let move_type = type_ref_is_move_type_with_enums(
                                        &field.ty,
                                        &classes,
                                        move_enum_names,
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
                                    }
                                })
                                .collect(),
                            returns_move_type: move_enum_names.contains(&declaration.name),
                            ..Signature::default()
                        },
                    );
                }
            }
            Item::Interface(_) | Item::Trait(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }

    let mut checker = Checker {
        classes,
        class_type_params,
        signatures,
        constructors,
        methods,
        enum_cases,
        properties,
        static_properties,
        inferred_move_returns: inferred_move_returns.clone(),
        return_borrows: return_borrows.clone(),
        resolved_types,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        exception_scopes: Vec::new(),
        move_enum_names: move_enum_names.clone(),
        receiver_class: None,
        receiver_writable: false,
        current_type_params: Vec::new(),
        current_return_borrow: None,
        active_assignment_writes: HashSet::new(),
        active_assignment_targets: HashSet::new(),
        active_borrows: Vec::new(),
        when_result_modes: Vec::new(),
        flow_facts,
        binding_resolution,
        closures,
        callable_value_calls,
        next_binding_id: 0,
        diagnostics: Vec::new(),
        closure_ownership: HashMap::new(),
        closure_values: HashMap::new(),
        analyzed_closures: HashSet::new(),
        prepared_closure_evaluations: HashSet::new(),
    };
    let mut top_level_scopes = Scopes::new();
    let mut top_level_falls_through = true;
    for item in &program.items {
        match item {
            Item::Function(function) => checker.check_function(function, None),
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Property(property) => {
                            if let Some(initializer) = &property.initializer {
                                let previous_receiver =
                                    checker.receiver_class.replace(class.name.clone());
                                let mut scopes = Scopes::new();
                                let function_value =
                                    checker.prepare_function_value(initializer, &mut scopes);
                                let function_storage_valid =
                                    function_value.as_ref().is_none_or(|value| {
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
                                if type_ref_is_move_type_with_enums(
                                    &property.ty,
                                    &checker.classes,
                                    &checker.move_enum_names,
                                    Some(&class.name),
                                ) || type_ref_mentions_parameter(
                                    &property.ty,
                                    &class.type_params,
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
                                checker.receiver_class = previous_receiver;
                            }
                        }
                        ClassMember::Method(method) => {
                            checker.check_function(method, Some(&class.name))
                        }
                        ClassMember::Constant(_) => {}
                    }
                }
            }
            Item::Enum(_) | Item::Interface(_) | Item::Trait(_) | Item::Constant(_) => {}
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
    }
}

pub(crate) fn function_return_borrow_in_context(
    function: &ast::FunctionDecl,
    enclosing_type_params: &[ast::TypeParamDecl],
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
) -> Option<ReturnBorrow> {
    if enclosing_type_params.is_empty() {
        return function_return_borrow_with_calls(function, resolve_call);
    }
    let mut scoped_function = function.clone();
    scoped_function
        .type_params
        .extend_from_slice(enclosing_type_params);
    function_return_borrow_with_calls(&scoped_function, resolve_call)
}

pub(crate) fn function_return_borrow_with_calls(
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
) -> Option<ReturnBorrow> {
    let mut borrow = None;
    if block_return_borrow(
        &function.body,
        function,
        resolve_call,
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
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
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
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &mut HashSet<String>,
    borrow: &mut Option<ReturnBorrow>,
) -> Option<bool> {
    match statement {
        Stmt::Block(block) => block_return_borrow(block, function, resolve_call, shadowed, borrow),
        Stmt::Return {
            expr: Some(expr), ..
        } => {
            let candidate = expr_return_borrow(expr, function, resolve_call, shadowed)?;
            match borrow {
                Some(existing) if existing.source != candidate.source => None,
                Some(existing) => {
                    existing.writable &= candidate.writable;
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
            if let Some(key) = &statement.key {
                loop_shadowed.insert(key.name.clone());
            }
            loop_shadowed.insert(statement.value.name.clone());
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
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
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
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    match expr {
        Expr::This { .. } if !function.is_static => Some(ReturnBorrow {
            source: BorrowSource::Receiver,
            writable: function.writable_this,
        }),
        Expr::Variable { name, .. }
            if !shadowed.contains(name)
                && (function.params.iter().any(|param| {
                    param.name == *name
                        && !param.take
                        && (type_ref_mentions_parameter(&param.ty, &function.type_params)
                            || matches!(
                                param.ty.name.as_str(),
                                "List" | "Dictionary" | "Set" | "Bytes" | "[]"
                            ))
                }) || function
                    .params
                    .iter()
                    .filter(|param| !param.take && param.ty.as_class_name().is_some())
                    .count()
                    == 1) =>
        {
            function
                .params
                .iter()
                .enumerate()
                .find(|(_, param)| param.name == *name && !param.take)
                .map(|(index, param)| ReturnBorrow {
                    source: BorrowSource::Parameter(index),
                    writable: param.writable,
                })
        }
        Expr::Grouped { expr, .. } => expr_return_borrow(expr, function, resolve_call, shadowed),
        Expr::Binary {
            left,
            op: BinaryOp::Coalesce,
            right,
            ..
        } => coalesced_return_borrow(left, right, function, resolve_call, shadowed),
        Expr::PropertyAccess { object, .. } => {
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
        Expr::FunctionCall { args, .. } | Expr::StaticCall { args, .. } => {
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
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
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
        (Some(mut left), Some(right), _, _) if left.source == right.source => {
            left.writable &= right.writable;
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
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    let returned = resolve_call(call)?;
    let source = match returned.source {
        BorrowSource::Receiver => receiver?,
        BorrowSource::Parameter(index) => {
            &argument_bound_to_parameter(function, args, index)?.value
        }
    };
    expr_return_borrow(source, function, resolve_call, shadowed).map(|mut borrow| {
        borrow.writable &= returned.writable;
        borrow
    })
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
    move_enum_names: &HashSet<String>,
    inferred_move_returns: &HashSet<usize>,
    return_borrows: &HashMap<usize, ReturnBorrow>,
    receiver_class: Option<&str>,
    enclosing_type_params: &[ast::TypeParamDecl],
) -> Signature {
    let return_borrow = return_borrows
        .get(&function.span.start)
        .copied()
        .or_else(|| {
            function_return_borrow_in_context(function, enclosing_type_params, &mut |_| None)
        })
        .filter(|_| {
            function.return_type.as_ref().is_some_and(|ty| {
                type_ref_class_name(ty, classes, receiver_class).is_some()
                    || type_ref_mentions_any_parameter(
                        ty,
                        &function.type_params,
                        enclosing_type_params,
                    )
            })
        });
    Signature {
        params: function
            .params
            .iter()
            .map(|param| Parameter {
                name: param.name.clone(),
                move_type: type_ref_is_move_type_with_enums(
                    &param.ty,
                    classes,
                    move_enum_names,
                    receiver_class,
                ),
                class_type: type_ref_class_name(&param.ty, classes, receiver_class).is_some(),
                generic: type_ref_mentions_any_parameter(
                    &param.ty,
                    &function.type_params,
                    enclosing_type_params,
                ),
                take: param.take,
                writable: param.writable,
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
                move_enum_names,
                receiver_class,
                &function.type_params,
                enclosing_type_params,
            )
        }),
        returns_move_type: function.return_type.as_ref().is_some_and(|ty| {
            (type_ref_is_move_type_with_enums(ty, classes, move_enum_names, receiver_class)
                || type_ref_mentions_potential_move_parameter(
                    ty,
                    &function.type_params,
                    enclosing_type_params,
                ))
                && return_borrow.is_none()
        }) || (function.return_type.is_none()
            && inferred_move_returns.contains(&function.span.start)),
        return_borrow,
        receiver: receiver_class.map(|_| {
            if function.writable_this {
                UseMode::Write
            } else {
                UseMode::Read
            }
        }),
    }
}

fn type_ref_mentions_parameter(ty: &crate::types::TypeRef, params: &[ast::TypeParamDecl]) -> bool {
    params.iter().any(|param| param.name == ty.name)
        || ty
            .type_arguments()
            .any(|argument| type_ref_mentions_parameter(argument, params))
}

fn type_ref_mentions_any_parameter(
    ty: &crate::types::TypeRef,
    function_params: &[ast::TypeParamDecl],
    enclosing_params: &[ast::TypeParamDecl],
) -> bool {
    type_ref_mentions_parameter(ty, function_params)
        || type_ref_mentions_parameter(ty, enclosing_params)
}

fn type_ref_mentions_potential_move_parameter(
    ty: &crate::types::TypeRef,
    function_params: &[ast::TypeParamDecl],
    enclosing_params: &[ast::TypeParamDecl],
) -> bool {
    function_params.iter().chain(enclosing_params).any(|param| {
        param.name == ty.name
            && !param.constraints.iter().any(|constraint| {
                matches!(
                    constraint.name.as_str(),
                    "Comparable" | "Equatable" | "Hashable"
                )
            })
    }) || ty.type_arguments().any(|argument| {
        type_ref_mentions_potential_move_parameter(argument, function_params, enclosing_params)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseMode {
    Read,
    Write,
    Give,
}

#[derive(Debug, Clone)]
struct ActiveBorrow {
    root: String,
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
    classes: HashSet<String>,
    class_type_params: HashMap<String, Vec<ast::TypeParamDecl>>,
    signatures: HashMap<String, Signature>,
    constructors: HashMap<String, Signature>,
    methods: HashMap<(String, String), Signature>,
    enum_cases: HashMap<(String, String), Signature>,
    properties: HashMap<(String, String), PropertyInfo>,
    static_properties: HashMap<(String, String), bool>,
    inferred_move_returns: HashSet<usize>,
    return_borrows: HashMap<usize, ReturnBorrow>,
    resolved_types: &'a HashMap<(usize, usize), crate::types::ResolvedType>,
    given_preludes: &'a HashMap<(usize, usize), crate::semantics::GivenSemanticInfo>,
    checked_effect_sites: &'a crate::checked_effects::EffectSiteMap,
    catch_error_types: &'a crate::checked_effects::CatchTypeMap,
    exception_scopes: Vec<ExceptionScope>,
    move_enum_names: HashSet<String>,
    receiver_class: Option<String>,
    receiver_writable: bool,
    current_type_params: Vec<ast::TypeParamDecl>,
    current_return_borrow: Option<UseMode>,
    active_assignment_writes: HashSet<String>,
    active_assignment_targets: HashSet<String>,
    active_borrows: Vec<ActiveBorrow>,
    when_result_modes: Vec<UseMode>,
    flow_facts: &'a FactsByUse,
    binding_resolution: &'a BindingResolution,
    closures: &'a HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    callable_value_calls: &'a HashMap<(usize, usize), crate::semantics::CallableValueCallInfo>,
    next_binding_id: usize,
    diagnostics: Vec<Diagnostic>,
    closure_ownership: HashMap<ClosureId, ClosureOwnershipInfo>,
    closure_values: HashMap<ClosureId, FunctionValueState>,
    analyzed_closures: HashSet<ClosureId>,
    prepared_closure_evaluations: HashSet<ClosureId>,
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
            .get(&(span.start, span.end))
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
    ) -> Option<FunctionValueState> {
        self.canonical_function_type(id)
            .map(|_| FunctionValueState {
                closure_id: None,
                provenance: if ownership == BindingOwnership::Owned {
                    ClosureValueProvenance::Owned
                } else {
                    ClosureValueProvenance::BorrowBound(
                        id.map(ClosureBorrowRoot::Binding).into_iter().collect(),
                    )
                },
                leases: Vec::new(),
                nonescaping_parameter: ownership != BindingOwnership::Owned,
                take_parameter_insertion,
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
        let Some(exception_scope) = self.exception_scopes.last_mut() else {
            return;
        };
        let effects = crate::checked_effects::effects_at(self.checked_effect_sites, span);
        for effect in effects {
            let mut exit_scopes = scopes.clone();
            exit_scopes.truncate_to(exception_scope.lexical_depth);
            exception_scope.exits.push(ExceptionalExit {
                effect: effect.clone(),
                scopes: exit_scopes,
            });
        }
    }

    fn propagate_exceptional_exits(&mut self, exits: impl IntoIterator<Item = ExceptionalExit>) {
        let Some(parent) = self.exception_scopes.last_mut() else {
            return;
        };
        for mut exit in exits {
            exit.scopes.truncate_to(parent.lexical_depth);
            parent.exits.push(exit);
        }
    }

    fn apply_finally_to_exceptional_exits(
        &mut self,
        finally: &ast::Block,
        exits: &mut Vec<ExceptionalExit>,
        return_move_type: bool,
    ) {
        let diagnostics_before = self.diagnostics.len();
        exits.retain_mut(|exit| {
            self.check_block(finally, &mut exit.scopes, return_move_type, true)
                .falls_through
        });
        self.deduplicate_diagnostics_from(diagnostics_before);
    }

    fn check_function(&mut self, function: &ast::FunctionDecl, receiver_class: Option<&str>) {
        let enclosing_type_params = receiver_class
            .and_then(|class| self.class_type_params.get(class))
            .cloned()
            .unwrap_or_default();
        let previous_receiver =
            std::mem::replace(&mut self.receiver_class, receiver_class.map(str::to_owned));
        let previous_receiver_writable =
            std::mem::replace(&mut self.receiver_writable, function.writable_this);
        let mut current_type_params = function.type_params.clone();
        current_type_params.extend(enclosing_type_params.clone());
        let previous_type_params =
            std::mem::replace(&mut self.current_type_params, current_type_params);
        let previous_return_borrow = self.current_return_borrow;
        self.current_return_borrow = function
            .return_type
            .as_ref()
            .is_some_and(|ty| {
                type_ref_class_name(ty, &self.classes, self.receiver_class.as_deref()).is_some()
                    || type_ref_mentions_any_parameter(
                        ty,
                        &function.type_params,
                        &enclosing_type_params,
                    )
            })
            .then(|| {
                self.return_borrows
                    .get(&function.span.start)
                    .copied()
                    .or_else(|| {
                        function_return_borrow_in_context(
                            function,
                            &enclosing_type_params,
                            &mut |_| None,
                        )
                    })
            })
            .flatten()
            .map(|borrow| {
                if borrow.writable {
                    UseMode::Write
                } else {
                    UseMode::Read
                }
            });
        let mut scopes = Scopes::new();
        for param in &function.params {
            let class =
                type_ref_class_name(&param.ty, &self.classes, self.receiver_class.as_deref());
            let mixed = param.ty.name == "mixed";
            let canonical_id = self.canonical_binding_id(param.span);
            let function_type = self.canonical_function_type(canonical_id);
            let ownership = if param.take {
                BindingOwnership::Owned
            } else if param.writable {
                BindingOwnership::WritableBorrow
            } else {
                BindingOwnership::ReadonlyBorrow
            };
            let function_value = self.parameter_function_value(
                canonical_id,
                ownership,
                (!param.take && !param.writable).then_some(param.ownership_modifier_insert),
            );
            scopes.declare(
                param.name.clone(),
                Binding {
                    id: self.next_binding_id(),
                    canonical_id,
                    class,
                    collection: type_ref_collection_info(
                        &param.ty,
                        &self.classes,
                        &self.move_enum_names,
                        self.receiver_class.as_deref(),
                        &self.current_type_params,
                        &[],
                    ),
                    mixed,
                    borrowed_place: !param.take,
                    borrow_root: None,
                    writable: param.writable,
                    state: if param.take && param.promoted_access.is_some() {
                        State::Given { at: param.span }
                    } else if param.take {
                        State::Owned
                    } else {
                        State::Borrowed
                    },
                    function_type,
                    function_value,
                    scope_depth: scopes.lexical_depth(),
                },
            );
        }
        let return_move_type = function.return_type.as_ref().is_some_and(|ty| {
            (type_ref_is_move_type_with_enums(
                ty,
                &self.classes,
                &self.move_enum_names,
                self.receiver_class.as_deref(),
            ) || type_ref_mentions_potential_move_parameter(ty, &self.current_type_params, &[]))
                && self.current_return_borrow.is_none()
        }) || (function.return_type.is_none()
            && self.inferred_move_returns.contains(&function.span.start));
        self.check_block(&function.body, &mut scopes, return_move_type, false);
        self.current_return_borrow = previous_return_borrow;
        self.current_type_params = previous_type_params;
        self.receiver_writable = previous_receiver_writable;
        self.receiver_class = previous_receiver;
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
            scopes.release_unused_closure_leases(&block.statements[index..], nested);
            let statement_flow = self.check_statement(statement, scopes, return_move_type);
            flow.falls_through = statement_flow.falls_through;
            flow.backedges.extend(statement_flow.backedges);
            flow.breaks.extend(statement_flow.breaks);
            flow.returns.extend(statement_flow.returns);
            flow.yields.extend(statement_flow.yields);
        }
        if nested {
            scopes.pop();
            for backedge in &mut flow.backedges {
                backedge.pop();
            }
            for break_exit in &mut flow.breaks {
                break_exit.pop();
            }
            for return_exit in &mut flow.returns {
                return_exit.pop();
            }
            for yield_exit in &mut flow.yields {
                yield_exit.pop();
            }
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
                let function_value = self.prepare_function_value(&decl.initializer, scopes);
                let borrowed_function_value = match ungroup_expr(&decl.initializer) {
                    Expr::Variable { name, .. } => scopes.get(name).is_some_and(|binding| {
                        binding.function_type.is_some()
                            && matches!(binding.state, State::Borrowed | State::BorrowedOrOwned)
                    }),
                    _ => false,
                };
                let borrowed_initializer =
                    self.expr_returns_borrow(&decl.initializer, scopes) || borrowed_function_value;
                let borrowed_mixed_index = borrowed_initializer
                    && self.expr_is_mixed_collection_index(&decl.initializer, scopes);
                let borrow_root = borrowed_initializer
                    .then(|| self.borrow_root_key(&decl.initializer, scopes))
                    .flatten();
                let initializer_moves = self.expr_is_move_value(&decl.initializer, scopes);
                let mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                    || (decl.ty.is_none()
                        && class.is_none()
                        && function_type.is_none()
                        && initializer_moves);
                let declared_move_type = decl.ty.as_ref().is_some_and(|ty| {
                    type_ref_is_move_type_with_enums(
                        ty,
                        &self.classes,
                        &self.move_enum_names,
                        self.receiver_class.as_deref(),
                    )
                });
                let borrowed_owning_value = borrowed_initializer
                    && !borrowed_mixed_index
                    && (initializer_moves || class.is_some() || mixed || declared_move_type);
                let invalid_borrow = borrowed_owning_value && borrow_root.is_none();
                let explicit_owning_borrow =
                    borrowed_initializer && declared_move_type && function_type.is_none();
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
                            "use an inferred readonly `let` binding for a borrow, or initialize this declaration with an independently owned value",
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
                if borrowed_owning_value && decl.writable {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0478",
                            format!("borrowed binding `${declaration_name}` cannot be writable"),
                            decl.span,
                        )
                        .with_help(
                            "remove `writable`; mutate through an explicitly writable borrowed parameter instead",
                        ),
                    );
                }
                let function_destination_valid = function_value.as_ref().is_none_or(|value| {
                    if decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed") {
                        self.reject_deferred_function_storage(
                            value,
                            decl.initializer.span(),
                            "`mixed`",
                            "Function Value Mixed Representation Is Not Yet Available",
                        )
                    } else {
                        self.validate_local_function_destination(
                            value,
                            scopes.lexical_depth(),
                            decl.initializer.span(),
                        )
                    }
                });
                self.use_expr(
                    &decl.initializer,
                    scopes,
                    if !function_destination_valid || borrowed_owning_value {
                        UseMode::Read
                    } else if initializer_moves || class.is_some() || mixed || declared_move_type {
                        UseMode::Give
                    } else {
                        UseMode::Read
                    },
                );
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
                                &self.move_enum_names,
                                self.receiver_class.as_deref(),
                                &self.current_type_params,
                                &[],
                            )
                        })
                        .or_else(|| self.expr_collection_info(&decl.initializer, scopes));
                    for (index, declaration) in decl.bindings.iter().enumerate() {
                        let canonical_id = self.canonical_binding_id(declaration.span);
                        scopes.declare(
                            declaration.name.clone(),
                            Binding {
                                id: self.next_binding_id(),
                                canonical_id,
                                class: class.clone(),
                                collection: collection.clone(),
                                mixed,
                                borrowed_place: borrowed_owning_value,
                                borrow_root: borrow_root.clone(),
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
                                function_value: (index == 0 && function_destination_valid)
                                    .then(|| function_value.clone())
                                    .flatten(),
                                scope_depth: scopes.lexical_depth(),
                            },
                        );
                    }
                }
                Flow::fallthrough()
            }
            Stmt::Assignment(assignment) => {
                if assignment.op != AssignOp::Assign {
                    self.use_assignment_operands(&assignment.target, &assignment.value, scopes);
                    return Flow::fallthrough();
                }
                if let Expr::Variable { name, span } = &assignment.target {
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
                    let value_is_function = self
                        .resolved_type(&assignment.value)
                        .and_then(non_null_function_type)
                        .is_some()
                        || self
                            .function_value_from_expr(&assignment.value, scopes)
                            .is_some();
                    let function_assignment = target_is_function || value_is_function;
                    let class_assignment = value_class.is_some()
                        && target
                            .as_ref()
                            .is_some_and(|binding| binding.mixed || binding.class == value_class);
                    let mixed_assignment = target.as_ref().is_some_and(|binding| binding.mixed);
                    let move_assignment = target.is_some() && value_moves;
                    if mixed_assignment && value_is_function {
                        if let Some(value) = self.prepare_function_value(&assignment.value, scopes)
                        {
                            self.reject_deferred_function_storage(
                                &value,
                                assignment.value.span(),
                                "`mixed`",
                                "Function Value Mixed Representation Is Not Yet Available",
                            );
                        }
                        self.use_expr(&assignment.value, scopes, UseMode::Read);
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
                        let pending_value = self.prepare_function_value(&assignment.value, scopes);
                        self.use_expr(
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
                        if let Some(binding) = scopes.get_mut(name) {
                            binding.state = State::Owned;
                            binding.function_value = pending_value;
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
                        self.use_expr(
                            &assignment.value,
                            scopes,
                            if value_moves || class_assignment {
                                UseMode::Give
                            } else {
                                UseMode::Read
                            },
                        );
                        if let Some(binding) = scopes.get_mut(name) {
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
                        if let Some(root) = self.borrow_root_key(&assignment.target, scopes) {
                            self.check_live_closure_conflict(
                                &root,
                                UseMode::Write,
                                assignment.target.span(),
                                scopes,
                            );
                            self.check_active_borrow_conflict(
                                &root,
                                UseMode::Write,
                                assignment.target.span(),
                            );
                        }
                        self.use_expr(&assignment.value, scopes, UseMode::Read);
                    }
                } else {
                    let static_function_storage =
                        matches!(ungroup_expr(&assignment.target), Expr::StaticMember { .. })
                            && self
                                .resolved_type(&assignment.target)
                                .and_then(non_null_function_type)
                                .is_some();
                    if static_function_storage {
                        if let Some(value) = self.prepare_function_value(&assignment.value, scopes)
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
                        let function_value = self.prepare_function_value(&assignment.value, scopes);
                        let valid_function_storage = function_value.as_ref().is_none_or(|value| {
                            self.validate_owned_function_storage(
                                value,
                                assignment.value.span(),
                                "an owned aggregate",
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
                        return Flow::fallthrough();
                    }
                    let property = self.assignment_property_info(&assignment.target, scopes);
                    let owning_property = property
                        .as_ref()
                        .is_some_and(|property| property.move_type || property.mixed);
                    let function_value = self.prepare_function_value(&assignment.value, scopes);
                    let mut valid_function_storage = true;
                    if let Some(value) = function_value.as_ref() {
                        let stores_on_receiver = matches!(
                            ungroup_expr(&assignment.target),
                            Expr::PropertyAccess { object, .. }
                                if matches!(ungroup_expr(object), Expr::This { .. })
                        );
                        if stores_on_receiver
                            && matches!(
                                &value.provenance,
                                ClosureValueProvenance::BorrowBound(roots)
                                    if roots.contains(&ClosureBorrowRoot::Receiver)
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
                            valid_function_storage = self.validate_owned_function_storage(
                                value,
                                assignment.value.span(),
                                "an instance property",
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
                }
                Flow::fallthrough()
            }
            Stmt::Echo { expr, .. } => {
                self.use_expr(expr, scopes, UseMode::Read);
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
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    if let Some(function_value) = self.prepare_function_value(expr, scopes) {
                        if !self.validate_returned_function_value(&function_value, expr.span()) {
                            self.use_expr(expr, scopes, UseMode::Read);
                            return Flow::stops();
                        }
                    }
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
                            self.use_expr(expr, scopes, mode);
                        }
                        return Flow::yields(scopes);
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
                }
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
                    scopes.pop();
                    pop_flow_scope(&mut flow);
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
                    scopes.pop();
                    pop_flow_scope(&mut flow);
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
                    if constant_bool(condition) == Some(false) {
                        scopes.pop();
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
                let mut returns = body_flow.returns;
                let mut exits = body_flow.backedges;
                exits.extend(body_flow.breaks);
                merge_loop_exit(scopes, &before, &exits);
                scopes.pop();
                for return_exit in &mut returns {
                    return_exit.pop();
                }
                Flow {
                    returns,
                    ..Flow::fallthrough()
                }
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
                if let Some(root) = self.borrow_root_key(&increment.target, scopes) {
                    self.check_live_closure_conflict(
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
                self.record_exceptional_exits(statement.span, scopes);
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
                    let catch_type = self
                        .catch_error_types
                        .get(&(catch.span.start, catch.span.end))
                        .expect("checked catch type");
                    let mut caught_states = Vec::new();
                    let mut remaining = Vec::new();
                    for exit in unmatched_exits {
                        if crate::checked_effects::effect_is_caught(&exit.effect, catch_type) {
                            caught_states.push(exit.scopes);
                        } else {
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
                                writable: false,
                                state: State::Owned,
                                function_type: None,
                                function_value: None,
                                scope_depth: catch_scopes.lexical_depth(),
                            },
                        );
                    }
                    let mut catch_flow =
                        self.check_block(&catch.body, &mut catch_scopes, return_move_type, false);
                    catch_scopes.pop();
                    pop_flow_scope(&mut catch_flow);
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
                    self.apply_finally_to_exceptional_exits(
                        &finally.body,
                        &mut propagating_exits,
                        return_move_type,
                    );
                    self.propagate_exceptional_exits(propagating_exits);
                    self.apply_finally_to_flow(
                        &finally.body,
                        scopes,
                        protected_flow,
                        return_move_type,
                    )
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
        mut flow: Flow,
        return_move_type: bool,
    ) -> Flow {
        if flow.falls_through {
            flow.falls_through = self
                .check_block(finally, scopes, return_move_type, true)
                .falls_through;
        }
        flow.backedges.retain_mut(|state| {
            self.check_block(finally, state, return_move_type, true)
                .falls_through
        });
        flow.breaks.retain_mut(|state| {
            self.check_block(finally, state, return_move_type, true)
                .falls_through
        });
        flow.returns.retain_mut(|state| {
            self.check_block(finally, state, return_move_type, true)
                .falls_through
        });
        flow.yields.retain_mut(|state| {
            self.check_block(finally, state, return_move_type, true)
                .falls_through
        });
        flow
    }

    fn check_given_setup<'a>(
        &mut self,
        given: &'a ast::GivenPrelude,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Vec<&'a Expr> {
        let predicate_indices = self
            .given_preludes
            .get(&(given.span.start, given.span.end))
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
        }
        self.check_second_iteration(&statement.body, &body_flow.backedges, return_move_type);
        let returns = body_flow.returns;
        let mut exits = body_flow.backedges;
        exits.extend(body_flow.breaks);
        merge_loop_exit(scopes, &before, &exits);
        Flow {
            returns,
            ..Flow::fallthrough()
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
        if let Some(key) = &statement.key {
            self.declare_foreach_binding(key, scopes);
        }
        self.declare_foreach_binding(&statement.value, scopes);
        let mut flow = self.check_block(&statement.body, scopes, return_move_type, false);
        scopes.pop();
        for backedge in &mut flow.backedges {
            backedge.pop();
        }
        for break_exit in &mut flow.breaks {
            break_exit.pop();
        }
        for return_exit in &mut flow.returns {
            return_exit.pop();
        }
        for yield_exit in &mut flow.yields {
            yield_exit.pop();
        }
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
                    &self.move_enum_names,
                    self.receiver_class.as_deref(),
                    &self.current_type_params,
                    &[],
                ),
                mixed: ty.name == "mixed",
                borrowed_place: true,
                borrow_root: None,
                writable: binding.writable,
                state: State::Borrowed,
                function_type: self.canonical_function_type(canonical_id),
                function_value: self.parameter_function_value(
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
                        &Stmt::Assignment(assignment.clone()),
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
        let assignment_root = tracked_target
            .then(|| self.borrow_root_key(target, scopes))
            .flatten();
        let inserted = assignment_root
            .as_ref()
            .is_some_and(|root| self.active_assignment_writes.insert(root.clone()));
        let assignment_target = tracked_target
            .then(|| self.assignment_place_key(target, scopes))
            .flatten();
        let target_inserted = assignment_target
            .as_ref()
            .is_some_and(|target| self.active_assignment_targets.insert(target.clone()));
        self.use_expr(value, scopes, value_mode);
        if target_inserted {
            self.active_assignment_targets
                .remove(assignment_target.as_deref().expect("inserted target"));
        }
        if inserted {
            self.active_assignment_writes
                .remove(assignment_root.as_deref().expect("inserted root"));
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
                .and_then(|(_, binding)| binding.function_value.clone());
            let source_is_move =
                resolved_type_is_move_type(&capture.source_type, &self.move_enum_names)
                    || resolved_type_requires_conservative_move(&capture.source_type);
            let (root, root_key, source_depth) = if declaration
                .is_some_and(|declaration| declaration.kind == BindingKind::MethodReceiver)
            {
                (ClosureBorrowRoot::Receiver, "$this".to_string(), 0)
            } else if let Some((name, binding)) = scopes.get_by_canonical(capture.source_binding_id)
            {
                let root = declaration
                    .and_then(|declaration| match declaration.owner {
                        crate::symbols::LexicalOwner::Closure(owner) => {
                            Some(ClosureBorrowRoot::EnclosingEnvironment(owner))
                        }
                        _ => None,
                    })
                    .unwrap_or(ClosureBorrowRoot::Binding(capture.source_binding_id));
                (root, binding_root(binding, name), binding.scope_depth)
            } else {
                (
                    ClosureBorrowRoot::Binding(capture.source_binding_id),
                    format!("binding:{}", capture.source_binding_id.0),
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
                CaptureAcquisitionKind::ReadonlyLease => Some(BorrowAccess::Readonly),
                CaptureAcquisitionKind::WritableLease => Some(BorrowAccess::Writable),
                CaptureAcquisitionKind::CopyIntoEnvironment
                | CaptureAcquisitionKind::MoveIntoEnvironment => None,
            };
            if let Some(access) = access {
                if let Some(conflict) = scopes.active_closure_leases().find(|lease| {
                    lease.root_key == root_key
                        && (access == BorrowAccess::Writable
                            || lease.access == BorrowAccess::Writable)
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
                        .with_related(conflict.capture_span, "Earlier Capture Remains Usable Here")
                        .with_help("finish using the earlier closure before creating this capture"),
                    );
                }
                if let Some(conflict) = self.active_borrows.iter().find(|borrow| {
                    borrow.root == root_key
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
                if access == BorrowAccess::Writable {
                    if let Some(alias) = scopes.borrowed_from(&root_key) {
                        invalid = true;
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0654",
                                format!("writable capture conflicts with live readonly binding `${alias}`"),
                                capture.declaration_span,
                            )
                            .with_title("Closure Capture Conflicts With Live Access")
                            .with_cause(cause.clone())
                            .with_help("finish using the readonly binding before creating this closure"),
                        );
                    }
                }
                leases.push(ClosureLease {
                    root: root.clone(),
                    root_key: root_key.clone(),
                    access,
                    capture_span: capture.declaration_span,
                    source_depth,
                });
            }

            if kind == CaptureAcquisitionKind::MoveIntoEnvironment {
                match scopes.get_by_canonical(capture.source_binding_id) {
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
                if let Some(conflict) = scopes
                    .active_closure_leases()
                    .find(|lease| lease.root_key == root_key)
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
                        .with_related(conflict.capture_span, "Earlier Capture Remains Usable Here")
                        .with_help(
                            "finish using the earlier closure before transferring ownership",
                        ),
                    );
                }
                if let Some(conflict) = self
                    .active_borrows
                    .iter()
                    .find(|borrow| borrow.root == root_key)
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
                if let Some(function_value) = source_function_value.as_ref() {
                    for lease in &function_value.leases {
                        if !leases.iter().any(|candidate| {
                            candidate.root == lease.root && candidate.access == lease.access
                        }) {
                            leases.push(lease.clone());
                        }
                    }
                }
            }

            acquisitions.push(CaptureAcquisition {
                environment_binding_id: capture.environment_binding_id,
                source_binding_id: capture.source_binding_id,
                kind,
                source_type: capture.source_type.clone(),
                roots: if access.is_some() {
                    vec![root]
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
        for source in move_sources {
            if let Some(binding) = scopes.get_mut_by_canonical(source) {
                binding.state = State::Given {
                    at: semantic.execution_boundary_span,
                };
            }
        }
        let mut roots = leases
            .iter()
            .map(|lease| lease.root.clone())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        let provenance = if roots.is_empty() {
            ClosureValueProvenance::Owned
        } else {
            ClosureValueProvenance::BorrowBound(roots)
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
            FunctionValueState {
                closure_id: Some(closure_id),
                provenance,
                leases,
                nonescaping_parameter: false,
                take_parameter_insertion: None,
            },
        );
    }

    fn function_value_from_expr(&self, expr: &Expr, scopes: &Scopes) -> Option<FunctionValueState> {
        match expr {
            Expr::Closure(closure) => self
                .closure_values
                .get(&ClosureId::from_span(closure.span))
                .cloned(),
            Expr::Variable { name, .. } => scopes
                .get(name)
                .and_then(|binding| binding.function_value.clone()),
            Expr::Grouped { expr, .. } => self.function_value_from_expr(expr, scopes),
            Expr::Null { .. } => None,
            _ => self
                .resolved_type(expr)
                .and_then(non_null_function_type)
                .map(|_| {
                    let root = self.function_borrow_root(expr, scopes).or_else(|| {
                        self.expr_returns_borrow(expr, scopes)
                            .then_some(ClosureBorrowRoot::Temporary)
                    });
                    let leases = root
                        .as_ref()
                        .map(|root| ClosureLease {
                            root: root.clone(),
                            root_key: self
                                .borrow_root_key(expr, scopes)
                                .unwrap_or_else(|| "temporary".to_string()),
                            access: BorrowAccess::Readonly,
                            capture_span: expr.span(),
                            source_depth: self.function_borrow_source_depth(expr, scopes),
                        })
                        .into_iter()
                        .collect::<Vec<_>>();
                    FunctionValueState {
                        closure_id: None,
                        provenance: root.map_or(ClosureValueProvenance::Owned, |root| {
                            ClosureValueProvenance::BorrowBound(vec![root])
                        }),
                        leases,
                        nonescaping_parameter: false,
                        take_parameter_insertion: None,
                    }
                }),
        }
    }

    fn function_borrow_root(&self, expr: &Expr, scopes: &Scopes) -> Option<ClosureBorrowRoot> {
        if !self.expr_returns_borrow(expr, scopes) {
            return None;
        }
        match ungroup_expr(expr) {
            Expr::Variable { name, .. } => scopes
                .get(name)
                .and_then(|binding| binding.canonical_id)
                .map(ClosureBorrowRoot::Binding),
            Expr::This { .. } => Some(ClosureBorrowRoot::Receiver),
            Expr::FunctionCall { name, args, .. } => {
                let signature = self.signatures.get(name)?;
                let borrow = signature.return_borrow?;
                self.call_borrow_source_expr(borrow, None, signature, args)
                    .and_then(|source| {
                        self.function_borrow_root(source, scopes).or_else(|| {
                            match ungroup_expr(source) {
                                Expr::Variable { name, .. } => scopes
                                    .get(name)
                                    .and_then(|binding| binding.canonical_id)
                                    .map(ClosureBorrowRoot::Binding),
                                Expr::This { .. } => Some(ClosureBorrowRoot::Receiver),
                                _ => None,
                            }
                        })
                    })
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                let class = self.expr_class(object, scopes)?;
                let signature = self.methods.get(&(class, method.clone()))?;
                let borrow = signature.return_borrow?;
                self.call_borrow_source_expr(borrow, Some(object), signature, args)
                    .and_then(|source| match ungroup_expr(source) {
                        Expr::Variable { name, .. } => scopes
                            .get(name)
                            .and_then(|binding| binding.canonical_id)
                            .map(ClosureBorrowRoot::Binding),
                        Expr::This { .. } => Some(ClosureBorrowRoot::Receiver),
                        _ => None,
                    })
            }
            Expr::CallableCall { args, span, .. } => {
                let call = self.callable_value_calls.get(&(span.start, span.end))?;
                let function = non_null_function_type(&call.function_type)?;
                let borrow = function.return_borrow?;
                let crate::types::FunctionBorrowSource::Parameter(index) = borrow.source;
                let source = &args.get(index)?.value;
                match ungroup_expr(source) {
                    Expr::Variable { name, .. } => scopes
                        .get(name)
                        .and_then(|binding| binding.canonical_id)
                        .map(ClosureBorrowRoot::Binding),
                    Expr::This { .. } => Some(ClosureBorrowRoot::Receiver),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn function_borrow_source_depth(&self, expr: &Expr, scopes: &Scopes) -> usize {
        self.function_borrow_root(expr, scopes)
            .and_then(|root| match root {
                ClosureBorrowRoot::Binding(id) => scopes
                    .get_by_canonical(id)
                    .map(|(_, binding)| binding.scope_depth),
                ClosureBorrowRoot::Receiver => Some(0),
                ClosureBorrowRoot::EnclosingEnvironment(_) | ClosureBorrowRoot::Temporary => None,
            })
            .unwrap_or_else(|| scopes.lexical_depth())
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

    fn prepare_function_value(
        &mut self,
        expr: &Expr,
        scopes: &mut Scopes,
    ) -> Option<FunctionValueState> {
        if let Expr::Closure(closure) = ungroup_expr(expr) {
            self.acquire_closure(closure, scopes);
            self.prepared_closure_evaluations
                .insert(ClosureId::from_span(closure.span));
        }
        self.function_value_from_expr(expr, scopes)
    }

    fn validate_local_function_destination(
        &mut self,
        value: &FunctionValueState,
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

    fn validate_returned_function_value(&mut self, value: &FunctionValueState, span: Span) -> bool {
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
        let ClosureValueProvenance::BorrowBound(roots) = &value.provenance else {
            self.mark_closure_escape(value, ClosureEscapeClassification::Owned);
            return true;
        };
        let mut accepted_root = None;
        for root in roots {
            let accepted = match root {
                ClosureBorrowRoot::Receiver => true,
                ClosureBorrowRoot::Binding(id) => self
                    .binding_resolution
                    .declarations_by_id
                    .get(id)
                    .is_some_and(|declaration| {
                        matches!(
                            declaration.kind,
                            BindingKind::FunctionParameter | BindingKind::MethodParameter
                        ) && declaration.ownership != BindingOwnership::Owned
                    }),
                ClosureBorrowRoot::EnclosingEnvironment(_) | ClosureBorrowRoot::Temporary => false,
            };
            if !accepted {
                self.diagnostics.push(self.with_function_value_cause(
                    Diagnostic::new(
                        "E0658",
                        "closure cannot return while borrowing a local or temporary value",
                        span,
                    )
                    .with_title("Closure Cannot Outlive Captured Value")
                    .with_help("capture an independently owned value with `take` before returning the closure"),
                    value,
                ));
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
        self.mark_closure_escape(value, ClosureEscapeClassification::ReturnedBorrow);
        true
    }

    fn mark_closure_escape(
        &mut self,
        value: &FunctionValueState,
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
        value: &FunctionValueState,
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
        value: &FunctionValueState,
    ) -> Diagnostic {
        let Some(ownership) = value
            .closure_id
            .and_then(|closure_id| self.closure_ownership.get(&closure_id))
        else {
            return diagnostic;
        };
        if !matches!(ownership.provenance, ClosureValueProvenance::BorrowBound(_)) {
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
            if source.kind == BindingKind::MethodReceiver
                || ((resolved_type_is_move_type(&acquisition.source_type, &self.move_enum_names)
                    || resolved_type_requires_conservative_move(&acquisition.source_type))
                    && source.ownership != BindingOwnership::Owned)
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
        value: &FunctionValueState,
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
        value: &FunctionValueState,
        span: Span,
        destination: &str,
    ) -> bool {
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
        if matches!(value.provenance, ClosureValueProvenance::BorrowBound(_)) {
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

    fn reject_deferred_function_storage(
        &mut self,
        value: &FunctionValueState,
        span: Span,
        destination: &str,
        title: &str,
    ) -> bool {
        if !self.validate_owned_function_storage(value, span, destination) {
            return false;
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

    fn check_live_closure_conflict(
        &mut self,
        root: &str,
        requested: UseMode,
        span: Span,
        scopes: &Scopes,
    ) {
        let Some(lease) = scopes.active_closure_leases().find(|lease| {
            lease.root_key == root
                && (lease.access == BorrowAccess::Writable
                    || matches!(requested, UseMode::Write | UseMode::Give))
        }) else {
            return;
        };
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
        match expr {
            Expr::Variable { name, span } => {
                let root = scopes.get(name).map(|binding| binding_root(binding, name));
                if let Some(root) = root.as_deref() {
                    self.check_live_closure_conflict(root, mode, *span, scopes);
                }
                if matches!(mode, UseMode::Write | UseMode::Give) {
                    if let Some(alias) = root
                        .as_deref()
                        .and_then(|root| scopes.borrowed_from(root))
                        .filter(|alias| *alias != name)
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0477",
                                format!(
                                    "`${name}` cannot be used as writable while `${alias}` borrows from it"
                                ),
                                *span,
                            )
                            .with_help(
                                "finish using the borrowed binding or place it in a shorter lexical block before mutating its owner",
                            ),
                        );
                    }
                }
                if let Some(root) = root.as_deref() {
                    if matches!(mode, UseMode::Read | UseMode::Write) {
                        self.check_active_borrow_conflict(root, mode, *span);
                    } else if mode == UseMode::Give {
                        self.check_give_against_active_borrows(root, *span);
                    }
                }
                if root.as_deref().is_some_and(|root| {
                    self.check_assignment_write_conflict(root, mode, *span) && mode == UseMode::Give
                }) {
                    return;
                }
                let nonescaping_value = (mode == UseMode::Give)
                    .then(|| {
                        scopes
                            .get(name)
                            .and_then(|binding| binding.function_value.as_ref())
                            .filter(|value| value.nonescaping_parameter)
                            .cloned()
                    })
                    .flatten();
                if let Some(value) = nonescaping_value {
                    let diagnostic = Diagnostic::new(
                        "E0657",
                        format!("nonescaping callback `${name}` cannot transfer ownership"),
                        *span,
                    )
                    .with_title("Nonescaping Callback Cannot Be Retained")
                    .with_help(
                        "declare the parameter with `take` when ownership must leave the call",
                    );
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
                            *span,
                        )
                        .with_title("Writable Invocation Requires Writable Access")
                        .with_help("declare the function-value binding writable")
                    } else {
                        Diagnostic::new(
                            "E0479",
                            format!("readonly `${name}` cannot be used as writable"),
                            *span,
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
                                *span,
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
                                *span,
                            )
                            .with_help(
                                "keep borrowed and owned values in separate bindings before transferring ownership",
                            ),
                        );
                    }
                    State::BorrowedOrOwned => {}
                    State::Owned if mode == UseMode::Give => {
                        binding.state = State::Given { at: *span };
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
                                *span,
                            )
                            .with_title(title)
                            .with_primary_label("Function Value Used Again Here")
                            .with_related(at, "Function Value Consumed Here")
                            .with_help("move or invoke a function value only once");
                            if let Some(closure_id) = binding
                                .function_value
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
                                *span,
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
            Expr::Grouped { expr, .. } => self.use_expr(expr, scopes, mode),
            Expr::PropertyAccess { object, span, .. } => {
                if mode == UseMode::Write {
                    if let Some(place) = self.assignment_place_key(expr, scopes) {
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
                let assignment_root = (matches!(mode, UseMode::Read | UseMode::Give)
                    && self
                        .assignment_place_key(expr, scopes)
                        .is_some_and(|target| self.active_assignment_targets.contains(&target)))
                .then(|| self.borrow_root_key(object, scopes))
                .flatten();
                let suspended_assignment_write = assignment_root
                    .as_ref()
                    .is_some_and(|root| self.active_assignment_writes.remove(root));
                self.use_expr(
                    object,
                    scopes,
                    if mode == UseMode::Write {
                        UseMode::Write
                    } else {
                        UseMode::Read
                    },
                );
                if suspended_assignment_write {
                    self.active_assignment_writes
                        .insert(assignment_root.expect("suspended assignment root"));
                }
            }
            Expr::FunctionCall { name, args, span } => {
                let signature = self.signatures.get(name).cloned().unwrap_or_default();
                self.use_call_args(None, args, &signature, CallExecution::Always, scopes);
                self.record_exceptional_exits(*span, scopes);
            }
            Expr::New {
                class_type,
                args,
                span,
                ..
            } => {
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
                    self.use_call_args(None, args, &signature, CallExecution::Always, scopes);
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
                if self
                    .callable_value_calls
                    .get(&(span.start, span.end))
                    .is_some_and(|call| {
                        call.target_kind == crate::semantics::CallableValueTargetKind::Property
                    })
                {
                    let property = Expr::PropertyAccess {
                        object: object.clone(),
                        property: method.clone(),
                        member_span: *member_span,
                        null_safe: false,
                        span: object.span().merge(*member_span),
                    };
                    self.use_callable_value_call(&property, args, *span, scopes);
                    return;
                }
                if let Some(collection) = self.expr_collection_info(object, scopes) {
                    self.use_collection_call(collection.family, object, method, args, scopes);
                    self.record_exceptional_exits(*span, scopes);
                    return;
                }
                let signature = self
                    .expr_class(object, scopes)
                    .and_then(|class| self.methods.get(&(class, method.clone())).cloned())
                    .unwrap_or_default();
                let execution = self.method_call_execution(*null_safe, object);
                self.use_call_args(Some(object), args, &signature, execution, scopes);
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
                            || (class_name == "Bytes" && method == "fromArray")
                ) {
                    for arg in args {
                        self.use_expr(&arg.value, scopes, UseMode::Read);
                    }
                    self.record_exceptional_exits(*span, scopes);
                    return;
                }
                let signature = self
                    .static_call_signature(qualifier, method)
                    .unwrap_or_default();
                let enum_payload = self
                    .qualifier_class(qualifier)
                    .is_some_and(|class| self.enum_cases.contains_key(&(class, method.clone())));
                if enum_payload {
                    self.use_call_args_with_storage(
                        None,
                        args,
                        &signature,
                        CallExecution::Always,
                        scopes,
                        Some(FunctionStorageBoundary::Owned("an enum payload")),
                    );
                } else {
                    self.use_call_args(None, args, &signature, CallExecution::Always, scopes);
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
                    let valid_function_storage = self
                        .prepare_function_value(&element.value, scopes)
                        .as_ref()
                        .is_none_or(|value| {
                            self.validate_owned_function_storage(
                                value,
                                element.value.span(),
                                "an owned aggregate",
                            )
                        });
                    let mode = if valid_function_storage {
                        self.use_owned_expression(&element.value, scopes)
                    } else {
                        self.use_expr(&element.value, scopes, UseMode::Read);
                        UseMode::Read
                    };
                    self.activate_place_input_borrows(&element.value, scopes);
                    if mode == UseMode::Read {
                        self.activate_borrow(&element.value, mode, scopes);
                    }
                }
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::ArrayRepeat { value, count, .. } => {
                let borrow_depth = self.active_borrows.len();
                let valid_function_storage = self
                    .prepare_function_value(value, scopes)
                    .as_ref()
                    .is_none_or(|function| {
                        self.validate_owned_function_storage(
                            function,
                            value.span(),
                            "an owned aggregate",
                        )
                    });
                let mode = if valid_function_storage {
                    self.use_owned_expression(value, scopes)
                } else {
                    self.use_expr(value, scopes, UseMode::Read);
                    UseMode::Read
                };
                self.activate_place_input_borrows(value, scopes);
                if mode == UseMode::Read {
                    self.activate_borrow(value, mode, scopes);
                }
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
                self.use_read_with_place_borrow(left, scopes);
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
                    Some(Fact::NonNull | Fact::Exact(_)) => {
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
                self.check_live_closure_conflict("$this", mode, *span, scopes);
                if matches!(mode, UseMode::Read | UseMode::Write) {
                    self.check_active_borrow_conflict("$this", mode, *span);
                } else if mode == UseMode::Give {
                    self.check_give_against_active_borrows("$this", *span);
                }
                if self.check_assignment_write_conflict("$this", mode, *span)
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
                ..
            } => self.use_match_expression(scrutinee, *match_mode, arms, scopes, mode),
            Expr::When(when) => {
                let has_given = when.given.is_some();
                if let Some(given) = &when.given {
                    scopes.push();
                    self.check_given_setup(given, scopes, false);
                }
                let before = scopes.clone();
                let mut outcomes = Vec::new();
                self.when_result_modes.push(mode);
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
                if let Some(first) = outcomes.first().cloned() {
                    let mut merged = first;
                    for outcome in outcomes.iter().skip(1) {
                        let current = merged.clone();
                        merged.merge_from(&current, outcome);
                    }
                    *scopes = merged;
                }
                if has_given {
                    scopes.pop();
                }
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
        let Some(call) = self
            .callable_value_calls
            .get(&(span.start, span.end))
            .cloned()
        else {
            return false;
        };
        let function_type = non_null_function_type(&call.function_type).cloned();
        let callee_mode = match call.invocation_mode {
            FunctionInvocationMode::Readonly => UseMode::Read,
            FunctionInvocationMode::Writable => UseMode::Write,
            FunctionInvocationMode::Once => UseMode::Give,
        };
        let stored_once_source = call.invocation_mode == FunctionInvocationMode::Once
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
                            .function_value
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
        if let Some(function_type) = function_type {
            self.use_callable_args(args, &function_type, scopes);
        } else {
            for argument in args {
                self.use_expr(&argument.value, scopes, UseMode::Read);
            }
        }
        self.record_exceptional_exits(span, scopes);
        true
    }

    fn use_match_expression(
        &mut self,
        scrutinee: &Expr,
        match_mode: ast::MatchMode,
        arms: &[ast::MatchArm],
        scopes: &mut Scopes,
        mode: UseMode,
    ) {
        let borrow_depth = self.active_borrows.len();
        let consuming = matches!(match_mode, ast::MatchMode::Consumed { .. });
        self.use_expr(
            scrutinee,
            scopes,
            if consuming {
                UseMode::Give
            } else {
                UseMode::Read
            },
        );
        let borrow_root = self.borrow_root_key(scrutinee, scopes);
        if !consuming {
            self.activate_place_borrow(scrutinee, UseMode::Read, scopes);
        }

        let mut remaining = scopes.clone();
        let mut outcomes = Vec::with_capacity(arms.len());
        let mut has_default = false;
        for arm in arms {
            if let ast::MatchPattern::Expression(pattern) = &arm.pattern {
                self.use_expr(pattern, &mut remaining, UseMode::Read);
            }

            let mut selected = remaining.clone();
            if let Some(guard) = &arm.guard {
                selected.push();
                for binding in match_pattern_bindings(&arm.pattern) {
                    self.declare_match_binding(binding, borrow_root.clone(), true, &mut selected);
                }
                self.use_expr(&guard.condition, &mut selected, UseMode::Read);
                selected.pop();

                let before_guard = remaining.clone();
                remaining.merge_from(&before_guard, &selected);
            }

            selected.push();
            for binding in match_pattern_bindings(&arm.pattern) {
                self.declare_match_binding(binding, borrow_root.clone(), !consuming, &mut selected);
            }
            self.use_expr(&arm.value, &mut selected, mode);
            selected.pop();
            outcomes.push(selected);

            if matches!(arm.pattern, ast::MatchPattern::Default { .. }) {
                has_default = true;
                break;
            }
        }

        if !has_default {
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
    }

    fn declare_match_binding(
        &mut self,
        binding: &ast::MatchBinding,
        borrow_root: Option<String>,
        borrowed: bool,
        scopes: &mut Scopes,
    ) {
        let Some(ty) = self
            .resolved_types
            .get(&(binding.span.start, binding.span.end))
        else {
            return;
        };
        let canonical_id = self.canonical_binding_id(binding.span);
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
                writable: false,
                state: if borrowed {
                    State::Borrowed
                } else {
                    State::Owned
                },
                function_type: non_null_function_type(ty).cloned(),
                function_value: self.parameter_function_value(
                    canonical_id,
                    if borrowed {
                        BindingOwnership::ReadonlyBorrow
                    } else {
                        BindingOwnership::Owned
                    },
                    None,
                ),
                scope_depth: scopes.lexical_depth(),
            },
        );
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
        let mut scopes = Scopes::new();
        for capture in &semantic.captures {
            let Some(declaration) = self
                .binding_resolution
                .declarations_by_id
                .get(&capture.environment_binding_id)
            else {
                continue;
            };
            if declaration.kind == BindingKind::MethodReceiver {
                continue;
            }
            let source_value = enclosing_scopes
                .get_by_canonical(capture.source_binding_id)
                .and_then(|(_, binding)| binding.function_value.clone());
            let borrowed = capture.mode != ast::ClosureCaptureMode::Take;
            let function_value = source_value.map(|mut value| {
                if borrowed {
                    value.provenance = ClosureValueProvenance::BorrowBound(vec![
                        ClosureBorrowRoot::EnclosingEnvironment(closure_id),
                    ]);
                    value.nonescaping_parameter = false;
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
                    borrow_root: borrowed
                        .then(|| format!("closure:{}:{}", closure_id.start, closure_id.end)),
                    writable: capture.mode == ast::ClosureCaptureMode::Writable,
                    state: if borrowed {
                        State::Borrowed
                    } else {
                        State::Owned
                    },
                    function_type: non_null_function_type(&capture.source_type).cloned(),
                    function_value,
                    scope_depth: scopes.lexical_depth(),
                },
            );
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
                    writable: parameter.writable,
                    state: if ownership == BindingOwnership::Owned {
                        State::Owned
                    } else {
                        State::Borrowed
                    },
                    function_type: non_null_function_type(source_type).cloned(),
                    function_value: self.parameter_function_value(
                        canonical_id,
                        ownership,
                        (!parameter.take && !parameter.writable).then_some(Span::new(
                            parameter.type_span.start,
                            parameter.type_span.start,
                        )),
                    ),
                    scope_depth: scopes.lexical_depth(),
                },
            );
        }

        let previous_return_borrow = self.current_return_borrow;
        self.current_return_borrow = non_null_function_type(&semantic.function_type)
            .and_then(|function| function.return_borrow)
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
        match &closure.body {
            ast::ClosureBody::Expression { expression, .. } => {
                if let Some(value) = self.prepare_function_value(expression, &mut scopes) {
                    let _ = self.validate_returned_function_value(&value, expression.span());
                }
                self.use_expr(
                    expression,
                    &mut scopes,
                    if return_move_type {
                        UseMode::Give
                    } else {
                        self.current_return_borrow.unwrap_or(UseMode::Read)
                    },
                );
            }
            ast::ClosureBody::Block(block) => {
                self.check_block(block, &mut scopes, return_move_type, false);
            }
        }
        self.current_return_borrow = previous_return_borrow;
    }

    fn use_callable_args(
        &mut self,
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
            self.use_expr(&argument.value, scopes, mode);
            self.activate_place_input_borrows(&argument.value, scopes);
            if matches!(mode, UseMode::Read | UseMode::Write) {
                self.activate_borrow(&argument.value, mode, scopes);
            }
        }
        self.active_borrows.truncate(borrow_depth);
    }

    fn use_call_args(
        &mut self,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        execution: CallExecution,
        scopes: &mut Scopes,
    ) {
        self.use_call_args_with_storage(receiver, args, signature, execution, scopes, None);
    }

    fn use_call_args_with_storage(
        &mut self,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        execution: CallExecution,
        scopes: &mut Scopes,
        function_storage: Option<FunctionStorageBoundary<'_>>,
    ) {
        let borrow_depth = self.active_borrows.len();
        if let Some(receiver) = receiver {
            self.use_expr(receiver, scopes, UseMode::Read);
            self.activate_place_input_borrows(receiver, scopes);
            if execution == CallExecution::Never {
                self.active_borrows.truncate(borrow_depth);
                return;
            }
            if let Some(mode) = signature.receiver {
                self.activate_borrow(receiver, mode, scopes);
            }
        }
        // Arguments are visited in source (written) order so ownership and
        // borrow conflicts are checked over the caller-visible evaluation order
        // (decision 0098). Each argument's ownership mode, however, comes from
        // the parameter it binds to by name, not its source position.
        let bound = signature.bind_arguments(args);
        let without_call = (execution == CallExecution::Maybe).then(|| scopes.clone());
        for (index, argument) in args.iter().enumerate() {
            let arg = &argument.value;
            let param_index = bound.arg_to_param.get(index).copied().flatten();
            let mut mode = param_index.map_or(UseMode::Read, |param| {
                self.call_arg_mode(signature, param, arg)
            });
            if mode == UseMode::Give {
                if let (Some(boundary), Some(value)) =
                    (function_storage, self.prepare_function_value(arg, scopes))
                {
                    let valid = match boundary {
                        FunctionStorageBoundary::Owned(destination) => {
                            self.validate_owned_function_storage(&value, arg.span(), destination)
                        }
                        FunctionStorageBoundary::Deferred { destination, title } => self
                            .reject_deferred_function_storage(
                                &value,
                                arg.span(),
                                destination,
                                title,
                            ),
                    };
                    if !valid {
                        mode = UseMode::Read;
                    }
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
                if let Some(root) = self.borrow_root_key(arg, scopes).filter(|root| {
                    self.active_borrows
                        .iter()
                        .skip(borrow_depth)
                        .any(|borrow| borrow.root == *root)
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
                if let Some(root) = self.borrow_root_key(arg, scopes) {
                    self.check_give_against_active_borrows(&root, arg.span());
                }
            }
            self.use_expr(arg, scopes, mode);
            self.activate_place_input_borrows(arg, scopes);
            if matches!(mode, UseMode::Read | UseMode::Write) {
                self.activate_borrow(arg, mode, scopes);
            }
        }
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
            Some(Fact::NonNull | Fact::Exact(_)) => CallExecution::Always,
            None => CallExecution::Maybe,
        }
    }

    fn flow_fact(&self, expr: &Expr) -> Option<&Fact> {
        let expr = ungroup_expr(expr);
        self.flow_facts.get(&(expr.span().start, expr.span().end))
    }

    fn activate_borrow(&mut self, expr: &Expr, mode: UseMode, scopes: &Scopes) {
        let Some(root) = self.borrow_root_key(expr, scopes) else {
            return;
        };
        self.activate_borrow_root(root, mode, expr.span(), scopes);
    }

    fn activate_place_borrow(&mut self, expr: &Expr, mode: UseMode, scopes: &Scopes) {
        let Some(root) = self
            .assignment_place_key(expr, scopes)
            .or_else(|| self.borrow_root_key(expr, scopes))
        else {
            return;
        };
        self.activate_borrow_root(root, mode, expr.span(), scopes);
    }

    fn activate_borrow_root(&mut self, root: String, mode: UseMode, span: Span, scopes: &Scopes) {
        if mode == UseMode::Write {
            if let Some(alias) = scopes.borrowed_from(&root) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0477",
                        format!(
                            "`{}` cannot be used as writable while `${alias}` borrows from it",
                            display_borrow_root(&root)
                        ),
                        span,
                    )
                    .with_help(
                        "finish using the borrowed binding or place it in a shorter lexical block before mutating its owner",
                    ),
                );
            }
        }
        self.check_active_borrow_conflict(&root, mode, span);
        self.active_borrows.push(ActiveBorrow { root, mode, span });
    }

    fn activate_place_input_borrows(&mut self, expr: &Expr, scopes: &Scopes) {
        match expr {
            Expr::Grouped { expr, .. } | Expr::PropertyAccess { object: expr, .. } => {
                self.activate_place_input_borrows(expr, scopes);
            }
            _ => self.activate_nested_property_borrows(expr, scopes),
        }
    }

    fn activate_nested_property_borrows(&mut self, expr: &Expr, scopes: &Scopes) {
        match expr {
            Expr::PropertyAccess { object, .. } => {
                if self
                    .borrow_root_key(expr, scopes)
                    .is_some_and(|root| !self.active_assignment_writes.contains(&root))
                {
                    self.activate_borrow(expr, UseMode::Read, scopes);
                }
                self.activate_nested_property_borrows(object, scopes);
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
                ..
            } => {
                let signature = self
                    .static_call_signature(qualifier, method)
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
        match expr {
            Expr::Grouped { expr, .. } => self.readonly_writable_path(expr, scopes),
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
                    .is_some_and(|property| !property.writable)
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
        let borrow = signature.return_borrow?;
        (!borrow.writable).then(|| ("returned borrow".to_string(), span))
    }

    fn check_assignment_write_conflict(&mut self, root: &str, mode: UseMode, span: Span) -> bool {
        if !self.active_assignment_writes.contains(root) {
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
                        "finish computing the property value before using the same owner again",
                    ),
                );
            }
        }
        true
    }

    fn borrow_root_key(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
        match expr {
            Expr::This { .. } if self.receiver_class.is_some() => Some("$this".to_string()),
            Expr::Variable { name, .. } => {
                scopes.get(name).map(|binding| binding_root(binding, name))
            }
            Expr::PropertyAccess { object, .. } | Expr::Grouped { expr: object, .. } => {
                self.borrow_root_key(object, scopes)
            }
            Expr::Index { collection, .. } => self.borrow_root_key(collection, scopes),
            Expr::StaticMember {
                qualifier, member, ..
            } => self.qualifier_class(qualifier).and_then(|class| {
                self.static_properties
                    .contains_key(&(class.clone(), member.clone()))
                    .then(|| format!("static:{class}::{member}"))
            }),
            Expr::FunctionCall { name, args, .. } => {
                let signature = self.signatures.get(name)?;
                let borrow = signature.return_borrow?;
                self.call_borrow_root(borrow, None, signature, args, scopes)
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                if self
                    .expr_collection_info(object, scopes)
                    .is_some_and(|collection| {
                        collection.family == CollectionFamily::Dictionary && method == "get"
                    })
                {
                    return self.borrow_root_key(object, scopes);
                }
                let class = self.expr_class(object, scopes)?;
                let signature = self.methods.get(&(class, method.clone()))?;
                let borrow = signature.return_borrow?;
                self.call_borrow_root(borrow, Some(object), signature, args, scopes)
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
                self.call_borrow_root(borrow, None, signature, args, scopes)
            }
            _ => None,
        }
    }

    fn assignment_place_key(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
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
            Expr::StaticMember { .. } => self.borrow_root_key(expr, scopes),
            _ => None,
        }
    }

    fn call_borrow_root(
        &self,
        borrow: ReturnBorrow,
        receiver: Option<&Expr>,
        signature: &Signature,
        args: &[Argument],
        scopes: &Scopes,
    ) -> Option<String> {
        match borrow.source {
            BorrowSource::Receiver => self.borrow_root_key(receiver?, scopes),
            BorrowSource::Parameter(index) => {
                // The annotation names a parameter position; named binding may
                // place that argument anywhere in the written call.
                let bound = signature.bind_arguments(args);
                let arg_index = bound.param_to_arg.get(index).copied().flatten()?;
                self.borrow_root_key(&args.get(arg_index)?.value, scopes)
            }
        }
    }

    fn check_active_borrow_conflict(&mut self, root: &str, mode: UseMode, span: Span) {
        if mode == UseMode::Give {
            self.check_give_against_active_borrows(root, span);
            return;
        }
        let Some(existing) = self
            .active_borrows
            .iter()
            .rev()
            .find(|borrow| borrow.root == root && borrow_modes_conflict(borrow.mode, mode))
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

    fn check_give_against_active_borrows(&mut self, root: &str, span: Span) {
        if let Some(existing) = self
            .active_borrows
            .iter()
            .rev()
            .find(|borrow| borrow.root == root)
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
                            || (class_name == "Bytes" && method == "fromArray")
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
                    return matches!(
                        collection.family,
                        CollectionFamily::List | CollectionFamily::Set
                    ) && matches!(property.as_str(), "first" | "last")
                        && collection.value_move;
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
            return matches!(
                collection.family,
                CollectionFamily::List | CollectionFamily::Set
            ) && matches!(property.as_str(), "first" | "last")
                && collection.value_move;
        }
        let Some(class) = self.expr_class(object, scopes) else {
            return false;
        };
        self.properties
            .get(&(class, property.clone()))
            .is_some_and(|property| property.move_type)
    }

    fn expr_returns_borrow(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => self.expr_returns_borrow(expr, scopes),
            Expr::FunctionCall { name, .. } => self
                .signatures
                .get(name)
                .is_some_and(|signature| signature.return_borrow.is_some()),
            Expr::MethodCall { object, method, .. } => {
                if let Some(collection) = self.expr_collection_info(object, scopes) {
                    return collection.value_move
                        && matches!(
                            (collection.family, method.as_str()),
                            (CollectionFamily::Dictionary, "get")
                        );
                }
                let Some(class) = self.expr_class(object, scopes) else {
                    return false;
                };
                self.methods
                    .get(&(class, method.clone()))
                    .is_some_and(|signature| signature.return_borrow.is_some())
            }
            Expr::StaticCall {
                qualifier, method, ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class| self.methods.get(&(class, method.clone())))
                .is_some_and(|signature| signature.return_borrow.is_some()),
            Expr::CallableCall { span, .. } => self
                .callable_value_calls
                .get(&(span.start, span.end))
                .and_then(|call| non_null_function_type(&call.function_type))
                .is_some_and(|function| function.return_borrow.is_some()),
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => self.expr_returns_borrow(left, scopes) || self.expr_returns_borrow(right, scopes),
            Expr::Index { collection, .. } => self
                .expr_collection_info(collection, scopes)
                .is_some_and(|collection| collection.value_move && !collection.value_mixed),
            Expr::PropertyAccess {
                object, property, ..
            } => self
                .expr_collection_info(object, scopes)
                .is_some_and(|collection| {
                    matches!(
                        collection.family,
                        CollectionFamily::List | CollectionFamily::Set
                    ) && collection.value_move
                        && matches!(property.as_str(), "first" | "last")
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
            } if class_name == "Bytes" && method == "fromArray" => Some(bytes_collection_info()),
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

    fn use_collection_call(
        &mut self,
        collection: CollectionFamily,
        object: &Expr,
        method: &str,
        args: &[Argument],
        scopes: &mut Scopes,
    ) {
        let mutating = matches!(
            (collection, method),
            (
                CollectionFamily::List,
                "add" | "insertAt" | "removeAt" | "pop" | "clear"
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

        for (index, argument) in args.iter().enumerate() {
            let moves_in = matches!(
                (collection, method, index),
                (CollectionFamily::List, "add", 0)
                    | (CollectionFamily::List, "insertAt", 1)
                    | (CollectionFamily::Dictionary, "set", 0 | 1)
                    | (CollectionFamily::Set, "add", 0)
                    | (CollectionFamily::PriorityQueue, "push", 0)
                    | (CollectionFamily::Deque, "pushFront" | "pushBack", 0)
            );
            if moves_in {
                let valid_function_storage = self
                    .prepare_function_value(&argument.value, scopes)
                    .as_ref()
                    .is_none_or(|value| {
                        self.validate_owned_function_storage(
                            value,
                            argument.value.span(),
                            "an owned collection",
                        )
                    });
                if valid_function_storage {
                    self.use_owned_expression(&argument.value, scopes);
                } else {
                    self.use_expr(&argument.value, scopes, UseMode::Read);
                }
            } else {
                self.use_expr(&argument.value, scopes, UseMode::Read);
            }
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
        qualifier: &ast::StaticQualifier,
        method: &str,
    ) -> Option<Signature> {
        let class = self.qualifier_class(qualifier)?;
        self.enum_cases
            .get(&(class.clone(), method.to_string()))
            .or_else(|| self.methods.get(&(class, method.to_string())))
            .cloned()
    }

    fn resolved_type(&self, expr: &Expr) -> Option<&crate::types::ResolvedType> {
        self.resolved_types
            .get(&(expr.span().start, expr.span().end))
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

fn binding_root(binding: &Binding, name: &str) -> String {
    binding
        .borrow_root
        .clone()
        .unwrap_or_else(|| binding_identity_key(binding.id, name))
}

fn display_borrow_root(root: &str) -> String {
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
                || (statement.key.as_ref().is_none_or(|key| key.name != name)
                    && statement.value.name != name
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
    let name = if ty.name == "self" {
        receiver_class?
    } else {
        &ty.name
    };
    classes.contains(name).then(|| name.to_string())
}

fn resolved_type_class(ty: &crate::types::ResolvedType) -> Option<&str> {
    match ty {
        crate::types::ResolvedType::Class(class) => Some(&class.name),
        crate::types::ResolvedType::Nullable(inner) => resolved_type_class(inner),
        crate::types::ResolvedType::SharedHandle(kind, payload) if kind.forwards_payload() => {
            resolved_type_class(payload)
        }
        _ => None,
    }
}

fn resolved_type_is_move_type(
    ty: &crate::types::ResolvedType,
    move_enum_names: &HashSet<String>,
) -> bool {
    match ty {
        crate::types::ResolvedType::Bytes
        | crate::types::ResolvedType::Mixed
        | crate::types::ResolvedType::Function(_)
        | crate::types::ResolvedType::Class(_)
        | crate::types::ResolvedType::SharedHandle(_, _)
        | crate::types::ResolvedType::TypedArray(_)
        | crate::types::ResolvedType::List(_)
        | crate::types::ResolvedType::Dictionary(_, _)
        | crate::types::ResolvedType::SortedDictionary(_, _)
        | crate::types::ResolvedType::Set(_)
        | crate::types::ResolvedType::SortedSet(_)
        | crate::types::ResolvedType::PriorityQueue(_)
        | crate::types::ResolvedType::Deque(_) => true,
        crate::types::ResolvedType::Enum(enum_type) => move_enum_names.contains(&enum_type.name),
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

fn resolved_type_requires_conservative_move(ty: &crate::types::ResolvedType) -> bool {
    match ty {
        crate::types::ResolvedType::TypeParameter(_) | crate::types::ResolvedType::Unsupported => {
            true
        }
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
        crate::types::ResolvedType::Class(class) => class
            .arguments
            .iter()
            .any(resolved_type_requires_conservative_move),
        crate::types::ResolvedType::SharedHandle(_, payload) => {
            resolved_type_requires_conservative_move(payload)
        }
        _ => false,
    }
}

fn resolved_collection_info(
    ty: &crate::types::ResolvedType,
    move_enum_names: &HashSet<String>,
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
    move_enum_names: &HashSet<String>,
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
        value_move: type_ref_is_move_type_with_enums(
            value,
            classes,
            move_enum_names,
            receiver_class,
        ) || type_ref_mentions_potential_move_parameter(
            value,
            type_params,
            enclosing_type_params,
        ),
        value_mixed: value.name == "mixed",
        value_class: type_ref_class_name(value, classes, receiver_class),
        value_collection: type_ref_collection_info(
            value,
            classes,
            move_enum_names,
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
    type_ref_is_move_type_with_enums(ty, classes, &HashSet::new(), receiver_class)
}

fn type_ref_is_move_type_with_enums(
    ty: &crate::types::TypeRef,
    classes: &HashSet<String>,
    move_enum_names: &HashSet<String>,
    receiver_class: Option<&str>,
) -> bool {
    // Every Stage 25a handle and access object is a move type (record 0106):
    // plain assignment transfers the handle and never silently retains.
    ty.function.is_some()
        || crate::types::SharedHandleKind::from_source_name(&ty.name).is_some()
        || type_ref_class_name(ty, classes, receiver_class).is_some()
        || move_enum_names.contains(&ty.name)
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

fn pop_flow_scope(flow: &mut Flow) {
    for backedge in &mut flow.backedges {
        backedge.pop();
    }
    for break_exit in &mut flow.breaks {
        break_exit.pop();
    }
    for return_exit in &mut flow.returns {
        return_exit.pop();
    }
    for yield_exit in &mut flow.yields {
        yield_exit.pop();
    }
}
