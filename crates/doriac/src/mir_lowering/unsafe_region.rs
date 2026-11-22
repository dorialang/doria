//! Unsafe regions (Decision 0148 sections 22 and 23). Each unsafe context
//! written outside every other one lowers into blocks of its own: an entry
//! the host jumps to, the region's code, and a continuation after it. The
//! blocks stay ordinary MIR, so Cranelift and LLVM compile them inline,
//! while the region records where it starts, where it falls through, and
//! every other edge that leaves it, for the native companion library.

use super::*;
use crate::foreign::UnsafeContextFact;

pub(super) struct UnsafeRegionBuilder {
    fact: UnsafeContextFact,
    id: String,
    entry: mir::BlockId,
    /// The region's blocks, in creation order: every block created while
    /// it is active, except the landing blocks of the routes leaving it.
    pub(super) blocks: Vec<mir::BlockId>,
    /// The first scope the region owns. A route that cleans a scope below it
    /// leaves the region.
    scope_boundary: usize,
    continuation: Option<mir::BlockId>,
    result: Option<mir::LocalId>,
}

impl LoweringContext<'_> {
    /// The unsafe context written at `span`, when it is a region.
    pub(super) fn unsafe_region_fact(&self, span: Span) -> Option<UnsafeContextFact> {
        self.semantic_info.unsafe_contexts.get(&span).copied()
    }

    fn begin_unsafe_region(&mut self, fact: UnsafeContextFact) -> usize {
        assert!(
            self.active_unsafe_region.is_none(),
            "an unsafe region never contains another one"
        );
        let entry = self.create_block();
        self.terminate_current(mir::Terminator::Jump(entry));
        self.current_block = Some(entry);
        let id = region_identity(&fact, self);
        self.unsafe_regions.push(UnsafeRegionBuilder {
            fact,
            id,
            entry,
            blocks: vec![entry],
            scope_boundary: self.local_scopes.len(),
            continuation: None,
            result: None,
        });
        let index = self.unsafe_regions.len() - 1;
        self.active_unsafe_region = Some(index);
        index
    }

    /// Ends the active region. When its code can fall through, control
    /// continues in a block after every block of the region.
    fn end_unsafe_region(&mut self, index: usize) {
        debug_assert_eq!(self.active_unsafe_region, Some(index));
        self.active_unsafe_region = None;
        let continuation = self.create_block();
        if self.current_block.is_some() {
            self.terminate_current(mir::Terminator::Jump(continuation));
            self.current_block = Some(continuation);
        }
        self.unsafe_regions[index].continuation = Some(continuation);
    }

    /// Whether `route` leaves the active region: it cleans a scope the region
    /// does not own, and no finalizer begun inside the region takes it first.
    pub(super) fn route_leaves_unsafe_region(&self, route: &RoutedExit) -> bool {
        let Some(index) = self.active_unsafe_region else {
            return false;
        };
        let boundary = self.unsafe_regions[index].scope_boundary;
        let inner_finalizer = self.active_finalizers.len() > route.target_finalizer_depth
            && self
                .active_finalizers
                .last()
                .is_some_and(|id| self.finalizer_regions[id.0].scope_depth >= boundary);
        !inner_finalizer && route.cleanup_depth < boundary
    }

    /// A route out of the active region cleans the scopes the region owns
    /// inside it, then continues from a landing block outside it, where the
    /// host destroys its own values, crosses finalizers and reaches the
    /// route's destination in ordinary Doria order (section 23).
    pub(super) fn leave_unsafe_region(&mut self, route: RoutedExit, source_scope_depth: usize) {
        let index = self
            .active_unsafe_region
            .expect("a route leaves the region it is in");
        let boundary = self.unsafe_regions[index].scope_boundary;
        self.cleanup_scope_range(boundary, source_scope_depth);
        self.active_unsafe_region = None;
        let landing = self.create_block();
        self.active_unsafe_region = Some(index);
        self.note_unsafe_route(&route.exit);
        self.terminate_current(mir::Terminator::Jump(landing));
        self.current_block = Some(landing);
        self.active_unsafe_region = None;
        self.route_structured_exit(route, boundary);
        self.active_unsafe_region = Some(index);
    }

    /// Records how the current block leaves through a structured route, so
    /// a terminator that reaches outside the active region is classified.
    pub(super) fn note_unsafe_route(&mut self, exit: &StructuredExit) {
        if self.active_unsafe_region.is_none() {
            return;
        }
        let kind = match exit {
            StructuredExit::FunctionReturn { .. } => mir::UnsafeRegionExitKind::Return,
            StructuredExit::Break { .. } => mir::UnsafeRegionExitKind::Break,
            StructuredExit::Continue { .. } => mir::UnsafeRegionExitKind::Continue,
            StructuredExit::CheckedError { error, .. } => {
                mir::UnsafeRegionExitKind::CheckedError { error: *error }
            }
            StructuredExit::Normal { .. } | StructuredExit::WhenYield { .. } => return,
        };
        let source = self.current_block();
        self.unsafe_exit_routes.insert(source, kind);
    }
}

/// `unsafe { }`: the block's statements run as one region.
pub(super) fn block(
    fact: UnsafeContextFact,
    block: &hir::Block,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let region = context.begin_unsafe_region(fact);
    context.push_scope();
    let result = lower_statement_sequence(&block.statements, return_type, context);
    context.pop_scope();
    context.end_unsafe_region(region);
    result
}

/// `unsafe(<expression>);` as a statement.
pub(super) fn statement(
    expr: &hir::Expr,
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<()>> {
    let Some((fact, inner)) = expression_region(expr, context) else {
        return Ok(None);
    };
    let region = context.begin_unsafe_region(fact);
    let result = lower_expression_statement(inner, span, context);
    context.end_unsafe_region(region);
    result.map(Some)
}

/// `unsafe(<expression>)` as a value: the region evaluates the expression
/// into a result local, and the value after the region is that local.
pub(super) fn value(
    expr: &hir::Expr,
    expected: mir::Type,
    consume_result: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    let Some((fact, inner)) = expression_region(expr, context) else {
        return Ok(None);
    };
    let ty = context.expression_type(inner)?;
    let region = context.begin_unsafe_region(fact);
    let lowered: DiagnosticResult<(mir::LocalId, bool)> = (|| {
        materialize_nested_collection_places(inner, false, context)?;
        let value = if consume_result {
            lower_rvalue_as_expected(inner, ty, context)?
        } else {
            lower_rvalue_as_borrowed(inner, ty, context)?
        };
        // The result local owns what the region produced, and borrows what
        // it only reached, as every materialized value does.
        let owned = user_local_type_owns_value(ty) && !value.borrows_move_value();
        let result = context.declare_checked_call_slot(ty, owned);
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value,
        });
        Ok((result, owned))
    })();
    context.end_unsafe_region(region);
    let (result, owned) = lowered?;
    context.unsafe_regions[region].result = Some(result);
    // An owned value the caller only reads, and a string, are released when
    // the enclosing statement ends.
    track_checked_result(result, ty, owned && !consume_result, context);
    convert_call_result(
        result,
        ty,
        expected,
        owned && consume_result,
        expr.span(),
        context,
    )
    .map(Some)
}

/// The error for a lowering path about to evaluate the unsafe region at
/// `span` as plain parentheses. Every path reaches a region through
/// [`value`] or [`statement`] first, so this reports a path that skipped
/// them.
pub(super) fn bypassed(span: Span) -> Vec<Diagnostic> {
    vec![Diagnostic::internal_compiler_error(
        "the compiler lowered an unsafe expression without its region",
        "a MIR lowering path removed the parentheses of `unsafe(...)` before the unsafe region hook saw it",
        span,
    )]
}

fn expression_region<'e>(
    expr: &'e hir::Expr,
    context: &LoweringContext,
) -> Option<(UnsafeContextFact, &'e hir::Expr)> {
    let hir::Expr::Grouped { expr: inner, span } = expr else {
        return None;
    };
    let fact = context.unsafe_region_fact(*span)?;
    (fact.form == crate::foreign::UnsafeContextForm::Expression).then_some((fact, inner.as_ref()))
}

/// Section 23: a hash of package identity, source identity, span and type
/// arguments. Both lowerings of one specialization (its ordinary entry and
/// its direct entry) produce the same text, so they share one identity.
fn region_identity(fact: &UnsafeContextFact, context: &LoweringContext) -> String {
    let compilation = context
        .semantic_info
        .compilation_contexts
        .get(&fact.span.source)
        .unwrap_or(&context.semantic_info.compilation_context);
    crate::foreign::region_identity(compilation, fact.span, &context.type_substitutions)
}

/// The function's regions with their exits, once every block is final. A
/// region block that returns or propagates an Error leaves the function; a
/// block that reaches outside the region must have left through a recorded
/// break, continue, return or checked-error route.
pub(super) fn finish(
    builders: Vec<UnsafeRegionBuilder>,
    routes: &HashMap<mir::BlockId, mir::UnsafeRegionExitKind>,
    blocks: &[mir::BasicBlock],
) -> DiagnosticResult<Vec<mir::UnsafeRegion>> {
    let mut regions = Vec::with_capacity(builders.len());
    for builder in builders {
        let continuation = builder
            .continuation
            .expect("every unsafe region ends before its function does");
        let mut region = mir::UnsafeRegion {
            id: builder.id,
            form: builder.fact.form,
            keyword: builder.fact.keyword,
            span: builder.fact.span,
            entry: builder.entry,
            blocks: builder.blocks,
            continuation,
            result: builder.result,
            inputs: Vec::new(),
            outputs: Vec::new(),
            exits: Vec::new(),
        };
        for block in region.blocks.iter().map(|block| &blocks[block.0]) {
            let source = block.id;
            match &block.terminator {
                mir::Terminator::Return(_) | mir::Terminator::ReturnVoid => {
                    region.exits.push(mir::UnsafeRegionExit {
                        source,
                        kind: mir::UnsafeRegionExitKind::Return,
                        target: None,
                    });
                    continue;
                }
                mir::Terminator::PropagateError { error } => {
                    region.exits.push(mir::UnsafeRegionExit {
                        source,
                        kind: mir::UnsafeRegionExitKind::CheckedError { error: *error },
                        target: None,
                    });
                    continue;
                }
                _ => {}
            }
            for target in block.terminator.targets() {
                if region.contains(target) || target == continuation {
                    continue;
                }
                let Some(kind) = routes.get(&source).copied() else {
                    return Err(vec![Diagnostic::internal_compiler_error(
                        "the compiler could not classify an edge leaving an unsafe region",
                        format!(
                            "block {} of region {} jumps to block {} outside it without a recorded route",
                            source.0, region.id, target.0
                        ),
                        region.span,
                    )]);
                };
                region.exits.push(mir::UnsafeRegionExit {
                    source,
                    kind,
                    target: Some(target),
                });
            }
        }
        regions.push(region);
    }
    Ok(regions)
}

/// Every unsafe context recorded by checking inside an emitted function is
/// a region of some lowered function, unless it lies in a branch checking
/// found dead for the target (`Reachability::is_live`, the one owner of
/// that fact). A lowering path that evaluated a context without its region
/// fails here instead of running it as plain code.
pub(super) fn require_every_live_region(
    program: &mir::Program,
    semantic_info: &SemanticInfo,
) -> DiagnosticResult<()> {
    let recorded = program
        .functions
        .iter()
        .flat_map(|function| function.unsafe_regions.iter().map(|region| region.span))
        .collect::<HashSet<_>>();
    let mut missing = semantic_info
        .unsafe_contexts
        .values()
        .filter(|fact| !recorded.contains(&fact.span))
        .filter(|fact| semantic_info.reachable.is_live(fact.span))
        .filter(|fact| {
            program
                .functions
                .iter()
                .any(|function| function.source_span.contains(fact.span))
        })
        .map(|fact| fact.span)
        .collect::<Vec<_>>();
    missing.sort();
    if missing.is_empty() {
        return Ok(());
    }
    Err(missing
        .into_iter()
        .map(|span| {
            Diagnostic::internal_compiler_error(
                "the compiler lowered an unsafe context without its region",
                "an unsafe context in an emitted function and a live branch has no MIR region",
                span,
            )
        })
        .collect())
}

/// Each region's inputs and outputs (section 23), once every function is
/// lowered so each direct call's parameter modes are known: a local the
/// region touches is an input when code outside the region also touches it
/// or it is a parameter, and an output when the region also writes, moves
/// or destroys it. A region terminator that leaves the function counts as
/// code outside it. Inputs keep the order of first use inside the region.
pub(super) fn bind_values(program: &mut mir::Program) {
    use crate::mir::local_access::{statement_locals, terminator_locals, LocalAccess};
    let parameter_modes = program
        .functions
        .iter()
        .map(|function| (function.id, function.parameter_modes.clone()))
        .collect::<HashMap<_, _>>();
    let modes = |function: mir::FunctionId, position: usize| {
        parameter_modes
            .get(&function)
            .and_then(|modes| modes.get(position))
            .copied()
    };
    let collections = &program.collection_types;
    for function in &mut program.functions {
        let mut regions = std::mem::take(&mut function.unsafe_regions);
        for region in &mut regions {
            let mut outside = function.params.iter().copied().collect::<HashSet<_>>();
            let mut used = Vec::new();
            let mut seen = HashSet::new();
            let mut written = HashSet::new();
            for block in &function.blocks {
                let inside = region.contains(block.id);
                let mut note = |inside: bool, local: mir::LocalId, access: LocalAccess| {
                    if !inside {
                        outside.insert(local);
                        return;
                    }
                    if seen.insert(local) {
                        used.push(local);
                    }
                    if access != LocalAccess::Read {
                        written.insert(local);
                    }
                };
                for statement in &block.statements {
                    statement_locals(statement, &modes, &mut |local, access| {
                        note(inside, local, access)
                    });
                }
                // The host runs a terminator that leaves the function in its
                // own frame, so the locals it touches cross back.
                let terminator_inside =
                    inside && !mir::companion::leaves_function(&block.terminator);
                terminator_locals(&block.terminator, &modes, &mut |local, access| {
                    note(terminator_inside, local, access)
                });
            }
            let locals = &function.locals;
            let value = |local: mir::LocalId| {
                mir::UnsafeRegionValue::of(local, locals[local.0].ty, collections)
            };
            region.inputs = used
                .iter()
                .copied()
                .filter(|local| outside.contains(local))
                .map(value)
                .collect();
            region.outputs = region
                .inputs
                .iter()
                .copied()
                .filter(|input| written.contains(&input.local))
                .collect();
        }
        function.unsafe_regions = regions;
    }
}

/// The companion plan (sections 22 and 23): one entry per region identity
/// and the host-operation table. Lowerings of one region that disagree are
/// a compiler defect.
pub(super) fn plan_companion(program: &mut mir::Program) -> DiagnosticResult<()> {
    program.companion = mir::companion::plan(program).map_err(|error| {
        vec![Diagnostic::internal_compiler_error(
            "the compiler lowered one unsafe region two different ways",
            error.details,
            error.span,
        )]
    })?;
    Ok(())
}
