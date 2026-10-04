use std::collections::{HashMap, HashSet};

use crate::ast::{Block, ElseBranch, Expr, ForIncrement, ForInitializer, ForStmt, Stmt};
use crate::checked_effects::{CatchCoverage, CatchCoverageMap, CatchTypeMap, EffectSiteMap};
use crate::source::Span;
use crate::types::ResolvedType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GivenSemanticInfo {
    pub predicate_statement_indices: Vec<usize>,
}

pub type GivenSemanticInfoMap = HashMap<Span, GivenSemanticInfo>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Entry,
    Statement,
    Branch,
    LoopHeader,
    Break,
    Continue,
    ReturnExit,
    DivergeExit,
    FallthroughExit,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum NodeAction {
    None,
    Statement(Stmt),
    Expression(Expr),
    Assume { condition: Expr, truth: bool },
    ForInitializer(ForInitializer),
    ForIncrement(ForIncrement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub span: Span,
    pub action: NodeAction,
    pub repeatable: bool,
    pub predecessors: Vec<NodeId>,
    pub successors: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlFlowGraph {
    pub nodes: Vec<Node>,
    pub entry: NodeId,
    pub fallthrough_exit: NodeId,
}

impl ControlFlowGraph {
    fn new(span: Span) -> Self {
        let entry = NodeId(0);
        Self {
            nodes: vec![Node {
                id: entry,
                kind: NodeKind::Entry,
                span,
                action: NodeAction::None,
                repeatable: false,
                predecessors: Vec::new(),
                successors: Vec::new(),
            }],
            entry,
            fallthrough_exit: entry,
        }
    }

    fn add_node(
        &mut self,
        kind: NodeKind,
        span: Span,
        action: NodeAction,
        repeatable: bool,
    ) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            id,
            kind,
            span,
            action,
            repeatable,
            predecessors: Vec::new(),
            successors: Vec::new(),
        });
        id
    }

    fn add_edge(&mut self, from: NodeId, to: NodeId) {
        if !self.nodes[from.0].successors.contains(&to) {
            self.nodes[from.0].successors.push(to);
            self.nodes[from.0].successors.sort_unstable();
        }
        if !self.nodes[to.0].predecessors.contains(&from) {
            self.nodes[to.0].predecessors.push(from);
            self.nodes[to.0].predecessors.sort_unstable();
        }
    }

    fn connect_all(&mut self, from: &[NodeId], to: NodeId) {
        for predecessor in from {
            self.add_edge(*predecessor, to);
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(kind: NodeKind, span: Span) -> Self {
        let mut graph = Self::new(span);
        graph.nodes[0].kind = kind;
        graph
    }

    #[cfg(test)]
    pub(crate) fn add_node_for_test(&mut self, kind: NodeKind, span: Span) -> NodeId {
        self.add_node(kind, span, NodeAction::None, false)
    }

    #[cfg(test)]
    pub(crate) fn add_edge_for_test(&mut self, from: NodeId, to: NodeId) {
        self.add_edge(from, to);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstantCondition {
    AlwaysTrue,
    AlwaysFalse,
    Unknown,
}

struct LoopContext {
    continue_target: NodeId,
    finalizer_depth: usize,
    breaks: Vec<NodeId>,
}

struct ExceptionHandler {
    catches: Vec<(Span, NodeId)>,
    catch_finalizer_depth: usize,
}

struct ValueRegion {
    finalizer_depth: usize,
    yields: Vec<NodeId>,
}

struct Builder<'a> {
    graph: ControlFlowGraph,
    loops: Vec<LoopContext>,
    finalizers: Vec<Block>,
    given_preludes: GivenSemanticInfoMap,
    checked_effect_sites: &'a EffectSiteMap,
    catch_error_types: &'a CatchTypeMap,
    catch_coverage: &'a CatchCoverageMap,
    terminal_expression_spans: &'a HashSet<Span>,
    exception_handlers: Vec<ExceptionHandler>,
    value_regions: Vec<ValueRegion>,
}

pub fn build_function_cfg(body: &Block, function_span: Span) -> ControlFlowGraph {
    build_function_cfg_with_given(body, function_span, &GivenSemanticInfoMap::new())
}

pub fn build_function_cfg_with_given(
    body: &Block,
    function_span: Span,
    given_preludes: &GivenSemanticInfoMap,
) -> ControlFlowGraph {
    let checked_effect_sites = EffectSiteMap::new();
    let catch_error_types = CatchTypeMap::new();
    let terminal_expression_spans = HashSet::new();
    build_function_cfg_with_checked_effects(
        body,
        function_span,
        given_preludes,
        &checked_effect_sites,
        &catch_error_types,
        &CatchCoverageMap::new(),
        &terminal_expression_spans,
    )
}

pub fn build_function_cfg_with_given_and_terminals(
    body: &Block,
    function_span: Span,
    given_preludes: &GivenSemanticInfoMap,
    terminal_expression_spans: &HashSet<Span>,
) -> ControlFlowGraph {
    let checked_effect_sites = EffectSiteMap::new();
    let catch_error_types = CatchTypeMap::new();
    build_function_cfg_with_checked_effects(
        body,
        function_span,
        given_preludes,
        &checked_effect_sites,
        &catch_error_types,
        &CatchCoverageMap::new(),
        terminal_expression_spans,
    )
}

pub(crate) fn build_function_cfg_with_checked_effects(
    body: &Block,
    function_span: Span,
    given_preludes: &GivenSemanticInfoMap,
    checked_effect_sites: &EffectSiteMap,
    catch_error_types: &CatchTypeMap,
    catch_coverage: &CatchCoverageMap,
    terminal_expression_spans: &HashSet<Span>,
) -> ControlFlowGraph {
    let graph = ControlFlowGraph::new(function_span);
    let entry = graph.entry;
    let mut builder = Builder {
        graph,
        loops: Vec::new(),
        finalizers: Vec::new(),
        given_preludes: given_preludes.clone(),
        checked_effect_sites,
        catch_error_types,
        catch_coverage,
        terminal_expression_spans,
        exception_handlers: Vec::new(),
        value_regions: Vec::new(),
    };
    let outgoing = builder.build_statements(&body.statements, vec![entry]);
    let fallthrough = builder.graph.add_node(
        NodeKind::FallthroughExit,
        body.span,
        NodeAction::None,
        false,
    );
    builder.graph.connect_all(&outgoing, fallthrough);
    builder.graph.fallthrough_exit = fallthrough;
    builder.graph
}

struct GivenFlow {
    setup_outgoing: Vec<NodeId>,
    predicates: Vec<Expr>,
}

struct GateFlow {
    entry: Option<NodeId>,
    passed: Vec<NodeId>,
    failed: Vec<NodeId>,
}

impl Builder<'_> {
    fn build_statements(&mut self, statements: &[Stmt], mut incoming: Vec<NodeId>) -> Vec<NodeId> {
        for statement in statements {
            incoming = self.build_statement(statement, incoming);
        }
        incoming
    }

    fn build_statement(&mut self, statement: &Stmt, incoming: Vec<NodeId>) -> Vec<NodeId> {
        match statement {
            Stmt::Block(block) => self.build_statements(&block.statements, incoming),
            Stmt::Return { span, .. } => {
                if let Some(finalizer_depth) = self
                    .value_regions
                    .last()
                    .map(|region| region.finalizer_depth)
                {
                    let action = match statement {
                        Stmt::Return {
                            expr: Some(expr), ..
                        } => NodeAction::Expression(expr.clone()),
                        _ => NodeAction::None,
                    };
                    let value = self.normal(NodeKind::Statement, *span, action, incoming);
                    let routed = self.route_finalizers(vec![value], finalizer_depth);
                    self.value_regions
                        .last_mut()
                        .expect("active value region")
                        .yields
                        .extend(routed);
                    return Vec::new();
                }
                let value = self.normal(
                    NodeKind::Statement,
                    *span,
                    NodeAction::Statement(statement.clone()),
                    incoming,
                );
                let routed = self.route_finalizers(vec![value], 0);
                self.terminal(NodeKind::ReturnExit, *span, NodeAction::None, routed);
                Vec::new()
            }
            Stmt::Throw(statement) => {
                let action = NodeAction::Expression(statement.expr.clone());
                let exclusions = action_evaluation_exclusions(&action);
                let incoming = self.prepare_action_evaluation(&action, incoming);
                let value = self.normal_without_checked_effects(
                    NodeKind::Statement,
                    statement.span,
                    NodeAction::Statement(Stmt::Throw(statement.clone())),
                    &incoming,
                );
                let has_checked_effect = self
                    .checked_effect_sites
                    .iter()
                    .any(|(span, effects)| statement.span.contains(*span) && !effects.is_empty());
                if has_checked_effect {
                    self.connect_checked_effects_except(
                        statement.span,
                        &incoming,
                        Some(value),
                        &exclusions,
                    );
                } else {
                    let routed = self.route_finalizers(vec![value], 0);
                    self.terminal(
                        NodeKind::DivergeExit,
                        statement.span,
                        NodeAction::None,
                        routed,
                    );
                }
                Vec::new()
            }
            Stmt::Try(statement) => {
                let precise_catches = statement
                    .catches
                    .iter()
                    .all(|catch| self.catch_error_types.contains_key(&catch.span));
                let catch_entries = if precise_catches {
                    statement
                        .catches
                        .iter()
                        .map(|catch| {
                            self.graph.add_node(
                                NodeKind::Statement,
                                catch.span,
                                NodeAction::None,
                                !self.loops.is_empty(),
                            )
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                if let Some(finally) = &statement.finally {
                    self.finalizers.push(finally.body.clone());
                }
                let before = incoming.clone();
                if precise_catches {
                    self.exception_handlers.push(ExceptionHandler {
                        catches: statement
                            .catches
                            .iter()
                            .zip(&catch_entries)
                            .map(|(catch, entry)| (catch.span, *entry))
                            .collect(),
                        catch_finalizer_depth: self.finalizers.len(),
                    });
                }
                let mut outgoing = self.build_statements(&statement.body.statements, incoming);
                if precise_catches {
                    self.exception_handlers
                        .pop()
                        .expect("checked try exception handler");
                }
                for (index, catch) in statement.catches.iter().enumerate() {
                    let catch_incoming = if precise_catches {
                        vec![catch_entries[index]]
                    } else {
                        before.clone()
                    };
                    outgoing.extend(self.build_statements(&catch.body.statements, catch_incoming));
                }
                let outgoing = deduplicate(outgoing);
                if let Some(finally) = &statement.finally {
                    self.finalizers.pop().expect("try finalizer context");
                    self.build_statements(&finally.body.statements, outgoing)
                } else {
                    outgoing
                }
            }
            Stmt::Expr { expr, span }
                if is_panic_call(expr) || self.terminal_expression_spans.contains(span) =>
            {
                self.terminal(
                    NodeKind::DivergeExit,
                    *span,
                    NodeAction::Statement(statement.clone()),
                    incoming,
                );
                Vec::new()
            }
            Stmt::If(if_stmt) => {
                if let Some(finally) = &if_stmt.finally {
                    self.finalizers.push(finally.block.clone());
                }
                let outgoing = if let Some(given) = &if_stmt.given {
                    let given = self.build_given(given, incoming);
                    let gate = self.build_gate(given.predicates, given.setup_outgoing, false);
                    self.build_if_with_gate(if_stmt, gate.passed, gate.failed)
                } else {
                    self.build_if(if_stmt, incoming)
                };
                if let Some(finally) = &if_stmt.finally {
                    self.finalizers.pop().expect("if finalizer context");
                    self.build_statements(&finally.block.statements, outgoing)
                } else {
                    outgoing
                }
            }
            Stmt::While(while_stmt) => {
                if let Some(finally) = &while_stmt.finally {
                    self.finalizers.push(finally.block.clone());
                }
                let gate = if let Some(given) = &while_stmt.given {
                    let given = self.build_given(given, incoming);
                    self.build_gate(given.predicates, given.setup_outgoing, true)
                } else {
                    GateFlow {
                        entry: None,
                        passed: incoming,
                        failed: Vec::new(),
                    }
                };
                let header = self.graph.add_node(
                    NodeKind::LoopHeader,
                    while_stmt.condition.span(),
                    NodeAction::None,
                    true,
                );
                self.graph.connect_all(&gate.passed, header);
                let predicate = self.normal(
                    NodeKind::Branch,
                    while_stmt.condition.span(),
                    NodeAction::Expression(while_stmt.condition.clone()),
                    vec![header],
                );
                let condition = constant_condition(&while_stmt.condition);
                self.loops.push(LoopContext {
                    continue_target: gate.entry.unwrap_or(header),
                    finalizer_depth: self.finalizers.len(),
                    breaks: Vec::new(),
                });
                let body_incoming = if condition == ConstantCondition::AlwaysFalse {
                    Vec::new()
                } else {
                    vec![self.assumption(&while_stmt.condition, true, predicate)]
                };
                let body_outgoing =
                    self.build_statements(&while_stmt.body.statements, body_incoming);
                self.graph
                    .connect_all(&body_outgoing, gate.entry.unwrap_or(header));
                let loop_context = self.loops.pop().expect("while loop context");
                let mut outgoing = loop_context.breaks;
                outgoing.extend(gate.failed);
                if condition != ConstantCondition::AlwaysTrue {
                    outgoing.push(self.assumption(&while_stmt.condition, false, predicate));
                }
                let outgoing = deduplicate(outgoing);
                if let Some(finally) = &while_stmt.finally {
                    self.finalizers.pop().expect("while finalizer context");
                    self.build_statements(&finally.block.statements, outgoing)
                } else {
                    outgoing
                }
            }
            Stmt::DoWhile(do_while) => {
                if let Some(finally) = &do_while.finally {
                    self.finalizers.push(finally.block.clone());
                }
                let body_entry = self.graph.add_node(
                    NodeKind::LoopHeader,
                    do_while.body.span,
                    NodeAction::None,
                    true,
                );
                self.graph.connect_all(&incoming, body_entry);
                let condition_node = self.graph.add_node(
                    NodeKind::LoopHeader,
                    do_while.condition.span(),
                    NodeAction::None,
                    true,
                );
                self.loops.push(LoopContext {
                    continue_target: condition_node,
                    finalizer_depth: self.finalizers.len(),
                    breaks: Vec::new(),
                });
                let body_outgoing =
                    self.build_statements(&do_while.body.statements, vec![body_entry]);
                self.graph.connect_all(&body_outgoing, condition_node);
                let predicate = self.normal(
                    NodeKind::Branch,
                    do_while.condition.span(),
                    NodeAction::Expression(do_while.condition.clone()),
                    vec![condition_node],
                );
                let condition = constant_condition(&do_while.condition);
                if condition != ConstantCondition::AlwaysFalse {
                    let repeat = self.assumption(&do_while.condition, true, predicate);
                    self.graph.add_edge(repeat, body_entry);
                }
                let mut outgoing = self.loops.pop().expect("do-while loop context").breaks;
                if condition != ConstantCondition::AlwaysTrue {
                    outgoing.push(self.assumption(&do_while.condition, false, predicate));
                }
                let outgoing = deduplicate(outgoing);
                if let Some(finally) = &do_while.finally {
                    self.finalizers.pop().expect("do-while finalizer context");
                    self.build_statements(&finally.block.statements, outgoing)
                } else {
                    outgoing
                }
            }
            Stmt::For(for_stmt) => self.build_for(for_stmt, incoming),
            Stmt::Foreach(foreach) => {
                let iterable = self.normal(
                    NodeKind::Statement,
                    foreach.iterable.span(),
                    NodeAction::Expression(foreach.iterable.clone()),
                    incoming,
                );
                let header = self.graph.add_node(
                    NodeKind::LoopHeader,
                    foreach.iterable.span(),
                    NodeAction::None,
                    true,
                );
                self.graph.add_edge(iterable, header);
                self.loops.push(LoopContext {
                    continue_target: header,
                    finalizer_depth: self.finalizers.len(),
                    breaks: Vec::new(),
                });
                let body_outgoing = self.build_statements(&foreach.body.statements, vec![header]);
                self.graph.connect_all(&body_outgoing, header);
                let mut outgoing = self.loops.pop().expect("foreach loop context").breaks;
                outgoing.push(header);
                deduplicate(outgoing)
            }
            Stmt::Break { span } => {
                let node = self.normal(NodeKind::Break, *span, NodeAction::None, incoming);
                if let Some(finalizer_depth) =
                    self.loops.last().map(|context| context.finalizer_depth)
                {
                    let routed = self.route_finalizers(vec![node], finalizer_depth);
                    self.loops
                        .last_mut()
                        .expect("break loop context")
                        .breaks
                        .extend(routed);
                }
                Vec::new()
            }
            Stmt::Continue { span } => {
                let node = self.normal(NodeKind::Continue, *span, NodeAction::None, incoming);
                if let Some((target, finalizer_depth)) = self
                    .loops
                    .last()
                    .map(|context| (context.continue_target, context.finalizer_depth))
                {
                    let routed = self.route_finalizers(vec![node], finalizer_depth);
                    self.graph.connect_all(&routed, target);
                }
                Vec::new()
            }
            _ => vec![self.normal(
                NodeKind::Statement,
                statement_span(statement),
                NodeAction::Statement(statement.clone()),
                incoming,
            )],
        }
    }

    fn build_given(
        &mut self,
        given: &crate::ast::GivenPrelude,
        incoming: Vec<NodeId>,
    ) -> GivenFlow {
        let predicate_indices = self
            .given_preludes
            .get(&given.span)
            .map(|info| info.predicate_statement_indices.as_slice())
            .unwrap_or_default()
            .to_vec();
        let mut setup_outgoing = incoming;
        let mut predicates = Vec::with_capacity(predicate_indices.len());
        for (index, statement) in given.block.statements.iter().enumerate() {
            if predicate_indices.contains(&index) {
                let Stmt::Expr { expr, .. } = statement else {
                    unreachable!("checked given predicate must be an expression statement")
                };
                predicates.push(expr.clone());
            } else {
                setup_outgoing = self.build_statement(statement, setup_outgoing);
            }
        }
        GivenFlow {
            setup_outgoing,
            predicates,
        }
    }

    fn build_gate(
        &mut self,
        predicates: Vec<Expr>,
        incoming: Vec<NodeId>,
        repeatable: bool,
    ) -> GateFlow {
        let mut passed = incoming;
        let mut failed = Vec::new();
        let mut entry = None;
        for predicate in predicates {
            let start = self.graph.add_node(
                NodeKind::Branch,
                predicate.span(),
                NodeAction::None,
                repeatable,
            );
            self.graph.connect_all(&passed, start);
            let branch = self.normal(
                NodeKind::Branch,
                predicate.span(),
                NodeAction::Expression(predicate.clone()),
                vec![start],
            );
            self.graph.nodes[branch.0].repeatable = repeatable;
            entry.get_or_insert(start);
            let condition = constant_condition(&predicate);
            passed = if condition == ConstantCondition::AlwaysFalse {
                Vec::new()
            } else {
                vec![self.assumption_with_repeatability(&predicate, true, branch, repeatable)]
            };
            if condition != ConstantCondition::AlwaysTrue {
                failed.push(
                    self.assumption_with_repeatability(&predicate, false, branch, repeatable),
                );
            }
        }
        GateFlow {
            entry,
            passed,
            failed,
        }
    }

    fn build_if(&mut self, if_stmt: &crate::ast::IfStmt, incoming: Vec<NodeId>) -> Vec<NodeId> {
        self.build_if_with_gate(if_stmt, incoming, Vec::new())
    }

    fn build_if_with_gate(
        &mut self,
        if_stmt: &crate::ast::IfStmt,
        incoming: Vec<NodeId>,
        gate_failed: Vec<NodeId>,
    ) -> Vec<NodeId> {
        let branch = self.normal(
            NodeKind::Branch,
            if_stmt.condition.span(),
            NodeAction::Expression(if_stmt.condition.clone()),
            incoming,
        );
        let condition = constant_condition(&if_stmt.condition);
        let then_incoming = if condition == ConstantCondition::AlwaysFalse {
            Vec::new()
        } else {
            vec![self.assumption(&if_stmt.condition, true, branch)]
        };
        let mut outgoing = self.build_statements(&if_stmt.then_block.statements, then_incoming);

        let else_incoming = if condition == ConstantCondition::AlwaysTrue {
            Vec::new()
        } else {
            vec![self.assumption(&if_stmt.condition, false, branch)]
        };
        match &if_stmt.else_branch {
            Some(ElseBranch::If(nested)) => {
                outgoing.extend(self.build_if_with_gate(nested, else_incoming, gate_failed));
            }
            Some(ElseBranch::Block(block)) => {
                let mut fallback_incoming = else_incoming;
                fallback_incoming.extend(gate_failed);
                outgoing.extend(self.build_statements(&block.statements, fallback_incoming));
            }
            None => {
                outgoing.extend(else_incoming);
                outgoing.extend(gate_failed);
            }
        }
        deduplicate(outgoing)
    }

    fn build_for(&mut self, for_stmt: &ForStmt, incoming: Vec<NodeId>) -> Vec<NodeId> {
        let mut incoming = incoming;
        if let Some(initializer) = &for_stmt.initializer {
            incoming = vec![self.normal(
                NodeKind::Statement,
                for_initializer_span(initializer),
                NodeAction::ForInitializer(initializer.clone()),
                incoming,
            )];
        }

        let header_span = for_stmt
            .condition
            .as_ref()
            .map(Expr::span)
            .unwrap_or(for_stmt.span);
        let header = self
            .graph
            .add_node(NodeKind::LoopHeader, header_span, NodeAction::None, true);
        self.graph.connect_all(&incoming, header);
        let predicate = for_stmt.condition.as_ref().map_or(header, |condition| {
            self.normal(
                NodeKind::Branch,
                condition.span(),
                NodeAction::Expression(condition.clone()),
                vec![header],
            )
        });
        let increment = for_stmt.increment.as_ref().map(|increment| {
            self.graph.add_node(
                NodeKind::Statement,
                for_increment_span(increment),
                NodeAction::None,
                true,
            )
        });
        let continue_target = increment.unwrap_or(header);
        self.loops.push(LoopContext {
            continue_target,
            finalizer_depth: self.finalizers.len(),
            breaks: Vec::new(),
        });

        let condition = for_stmt
            .condition
            .as_ref()
            .map(constant_condition)
            .unwrap_or(ConstantCondition::AlwaysTrue);
        let body_incoming = if condition == ConstantCondition::AlwaysFalse {
            Vec::new()
        } else {
            match &for_stmt.condition {
                Some(condition) => vec![self.assumption(condition, true, predicate)],
                None => vec![header],
            }
        };
        let body_outgoing = self.build_statements(&for_stmt.body.statements, body_incoming);
        if let (Some(action), Some(entry)) = (&for_stmt.increment, increment) {
            let completed = self.normal(
                NodeKind::Statement,
                for_increment_span(action),
                NodeAction::ForIncrement(action.clone()),
                vec![entry],
            );
            self.graph.add_edge(completed, header);
        }
        self.graph
            .connect_all(&body_outgoing, increment.unwrap_or(header));

        let mut outgoing = self.loops.pop().expect("for loop context").breaks;
        if condition != ConstantCondition::AlwaysTrue {
            if let Some(condition) = &for_stmt.condition {
                outgoing.push(self.assumption(condition, false, predicate));
            } else {
                outgoing.push(header);
            }
        }
        deduplicate(outgoing)
    }

    fn route_finalizers(&mut self, mut incoming: Vec<NodeId>, target_depth: usize) -> Vec<NodeId> {
        let active = self.finalizers.clone();
        for index in (target_depth..active.len()).rev() {
            self.finalizers.truncate(index);
            // An outer callable's finalizer is outside any inner value region.
            // A return there overrides propagation; it is not a when yield.
            let retained = self
                .value_regions
                .partition_point(|region| region.finalizer_depth <= index);
            let suspended = self.value_regions.split_off(retained);
            incoming = self.build_statements(&active[index].statements, incoming);
            self.value_regions.extend(suspended);
        }
        self.finalizers = active;
        incoming
    }

    /// Most expressions cannot write source bindings and remain one action.
    /// Value-producing control flow contains real statements, so its evaluation
    /// must be represented before the enclosing action commits its result.
    fn prepare_action_evaluation(
        &mut self,
        action: &NodeAction,
        incoming: Vec<NodeId>,
    ) -> Vec<NodeId> {
        let expressions = action_expressions(action);
        if !expressions.iter().any(|expr| contains_value_region(expr)) {
            return incoming;
        }
        if let Some(assignment) = action_assignment(action) {
            let incoming = expression_children(&assignment.target)
                .into_iter()
                .fold(incoming, |incoming, child| {
                    self.build_expression_evaluation(child, incoming)
                });
            let incoming = self.build_expression_evaluation(&assignment.value, incoming);
            self.connect_expression_effects(&assignment.target, &incoming);
            return incoming;
        }
        expressions
            .into_iter()
            .fold(incoming, |incoming, expression| {
                self.build_expression_evaluation(expression, incoming)
            })
    }

    fn build_expression_evaluation(
        &mut self,
        expression: &Expr,
        incoming: Vec<NodeId>,
    ) -> Vec<NodeId> {
        let outgoing = match expression {
            Expr::When(when) => self.build_when(when, incoming),
            Expr::Binary {
                left, op, right, ..
            } if matches!(op, crate::ast::BinaryOp::And | crate::ast::BinaryOp::Or) => {
                let outgoing = self.build_expression_evaluation(left, incoming);
                let branch = self.normal_without_checked_effects(
                    NodeKind::Branch,
                    left.span(),
                    NodeAction::None,
                    &outgoing,
                );
                let condition = constant_condition(left);
                let right_truth = matches!(op, crate::ast::BinaryOp::And);
                let mut outgoing = Vec::new();
                if condition == ConstantCondition::Unknown
                    || (condition == ConstantCondition::AlwaysTrue) == right_truth
                {
                    let right_incoming = self.assumption(left, right_truth, branch);
                    outgoing.extend(self.build_expression_evaluation(right, vec![right_incoming]));
                }
                if condition == ConstantCondition::Unknown
                    || (condition == ConstantCondition::AlwaysTrue) != right_truth
                {
                    outgoing.push(self.assumption(left, !right_truth, branch));
                }
                outgoing
            }
            Expr::Binary {
                left,
                op: crate::ast::BinaryOp::Coalesce,
                right,
                ..
            } => {
                let outgoing = self.build_expression_evaluation(left, incoming);
                // A present left value bypasses the right evaluation entirely.
                let mut joined = if matches!(left.as_ref(), Expr::Null { .. }) {
                    Vec::new()
                } else {
                    outgoing.clone()
                };
                joined.extend(self.build_expression_evaluation(right, outgoing));
                deduplicate(joined)
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                let incoming = self.build_expression_evaluation(scrutinee, incoming);
                let mut outgoing = Vec::new();
                for arm in arms {
                    let mut incoming = incoming.clone();
                    if let Some(guard) = &arm.guard {
                        incoming = self.build_expression_evaluation(&guard.condition, incoming);
                        if constant_condition(&guard.condition) == ConstantCondition::AlwaysFalse {
                            continue;
                        }
                    }
                    outgoing.extend(self.build_expression_evaluation(&arm.value, incoming));
                }
                deduplicate(outgoing)
            }
            _ => expression_children(expression)
                .into_iter()
                .fold(incoming, |incoming, child| {
                    self.build_expression_evaluation(child, incoming)
                }),
        };
        // Checked effects belong to the evaluation point, not to the enclosing
        // assignment's predecessor. Never execute a closure body at creation.
        if is_panic_call(expression) || self.terminal_expression_spans.contains(&expression.span())
        {
            return Vec::new();
        }
        self.connect_expression_effects(expression, &outgoing);
        outgoing
    }

    fn connect_expression_effects(&mut self, expression: &Expr, incoming: &[NodeId]) {
        let effects = self.checked_effect_sites.get(&expression.span()).cloned();
        if let Some(effects) = effects {
            for effect in effects {
                self.connect_checked_effect(&effect, expression.span(), incoming);
            }
        }
    }

    fn build_when(
        &mut self,
        when: &crate::ast::WhenExpression,
        incoming: Vec<NodeId>,
    ) -> Vec<NodeId> {
        let finalizer_depth = self.finalizers.len();
        self.value_regions.push(ValueRegion {
            finalizer_depth,
            yields: Vec::new(),
        });
        if let Some(finally) = &when.finally {
            self.finalizers.push(finally.block.clone());
        }
        let gate = if let Some(given) = &when.given {
            let given = self.build_given(given, incoming);
            self.build_gate(
                given.predicates,
                given.setup_outgoing,
                !self.loops.is_empty(),
            )
        } else {
            GateFlow {
                entry: None,
                passed: incoming,
                failed: Vec::new(),
            }
        };
        let mut remaining = gate.passed;
        let mut gate_failed = gate.failed;
        let mut fallthrough = Vec::new();
        for branch in &when.branches {
            let entered = if let Some(condition) = &branch.condition {
                let node = self.normal(
                    NodeKind::Branch,
                    condition.span(),
                    NodeAction::Expression(condition.clone()),
                    remaining,
                );
                let value = constant_condition(condition);
                remaining = if value == ConstantCondition::AlwaysTrue {
                    Vec::new()
                } else {
                    vec![self.assumption(condition, false, node)]
                };
                if value == ConstantCondition::AlwaysFalse {
                    Vec::new()
                } else {
                    vec![self.assumption(condition, true, node)]
                }
            } else {
                remaining.append(&mut gate_failed);
                std::mem::take(&mut remaining)
            };
            fallthrough.extend(self.build_statements(&branch.block.statements, entered));
        }
        fallthrough.extend(remaining);
        fallthrough.extend(gate_failed);
        if let Some(finally) = &when.finally {
            self.finalizers.pop().expect("when finalizer context");
            fallthrough = self.build_statements(&finally.block.statements, fallthrough);
        }
        let region = self.value_regions.pop().expect("when value region");
        fallthrough.extend(region.yields);
        deduplicate(fallthrough)
    }

    fn normal(
        &mut self,
        kind: NodeKind,
        span: Span,
        action: NodeAction,
        incoming: Vec<NodeId>,
    ) -> NodeId {
        let exclusions = action_evaluation_exclusions(&action);
        let incoming = self.prepare_action_evaluation(&action, incoming);
        let node = self.normal_without_checked_effects(kind, span, action, &incoming);
        self.connect_checked_effects_except(span, &incoming, None, &exclusions);
        node
    }

    fn normal_without_checked_effects(
        &mut self,
        kind: NodeKind,
        span: Span,
        action: NodeAction,
        incoming: &[NodeId],
    ) -> NodeId {
        let node = self
            .graph
            .add_node(kind, span, action, !self.loops.is_empty());
        self.graph.connect_all(incoming, node);
        node
    }

    fn connect_checked_effects_except(
        &mut self,
        action_span: Span,
        before_action: &[NodeId],
        completed_action: Option<NodeId>,
        exclusions: &[Span],
    ) {
        let sites = self
            .checked_effect_sites
            .iter()
            .filter(|(span, _)| action_span.contains(**span))
            .filter(|(span, _)| !exclusions.iter().any(|excluded| excluded.contains(**span)))
            .map(|(span, effects)| (*span, effects.clone()))
            .collect::<Vec<_>>();
        for (site_span, effects) in sites {
            let completed = completed_action.filter(|_| site_span == action_span);
            let sources = completed
                .map(|node| vec![node])
                .unwrap_or_else(|| before_action.to_vec());
            for effect in effects {
                self.connect_checked_effect(&effect, site_span, &sources);
            }
        }
    }

    fn connect_checked_effect(&mut self, effect: &ResolvedType, span: Span, sources: &[NodeId]) {
        let mut handlers = Vec::new();
        let mut fully_caught = false;
        'handlers: for handler in self.exception_handlers.iter().rev() {
            for (catch, target) in &handler.catches {
                let coverage =
                    crate::checked_effects::catch_coverage(self.catch_coverage, *catch, effect);
                if coverage != CatchCoverage::None {
                    handlers.push((*target, handler.catch_finalizer_depth));
                }
                if coverage == CatchCoverage::Complete {
                    fully_caught = true;
                    break 'handlers;
                }
            }
        }
        for (target, finalizer_depth) in handlers {
            let routed = self.route_finalizers(sources.to_vec(), finalizer_depth);
            self.graph.connect_all(&routed, target);
        }
        if !fully_caught {
            let routed = self.route_finalizers(sources.to_vec(), 0);
            self.terminal(NodeKind::DivergeExit, span, NodeAction::None, routed);
        }
    }

    fn assumption(&mut self, condition: &Expr, truth: bool, incoming: NodeId) -> NodeId {
        self.assumption_with_repeatability(condition, truth, incoming, !self.loops.is_empty())
    }

    fn assumption_with_repeatability(
        &mut self,
        condition: &Expr,
        truth: bool,
        incoming: NodeId,
        repeatable: bool,
    ) -> NodeId {
        let node = self.graph.add_node(
            NodeKind::Branch,
            condition.span(),
            NodeAction::Assume {
                condition: condition.clone(),
                truth,
            },
            repeatable,
        );
        self.graph.add_edge(incoming, node);
        node
    }

    fn terminal(&mut self, kind: NodeKind, span: Span, action: NodeAction, incoming: Vec<NodeId>) {
        self.normal_without_checked_effects(kind, span, action, &incoming);
    }
}

fn constant_condition(expr: &Expr) -> ConstantCondition {
    match expr {
        Expr::Bool { value: true, .. } => ConstantCondition::AlwaysTrue,
        Expr::Bool { value: false, .. } => ConstantCondition::AlwaysFalse,
        Expr::Grouped { expr, .. } => constant_condition(expr),
        _ => ConstantCondition::Unknown,
    }
}

fn is_panic_call(expr: &Expr) -> bool {
    matches!(expr, Expr::FunctionCall { name, .. } if name == "panic")
}

fn action_assignment(action: &NodeAction) -> Option<&crate::ast::Assignment> {
    match action {
        NodeAction::Statement(Stmt::Assignment(assignment))
        | NodeAction::ForInitializer(ForInitializer::Assignment(assignment)) => Some(assignment),
        NodeAction::ForIncrement(ForIncrement::Assignment(assignment)) => Some(assignment),
        _ => None,
    }
}

fn action_expressions(action: &NodeAction) -> Vec<&Expr> {
    if let Some(assignment) = action_assignment(action) {
        return vec![&assignment.target, &assignment.value];
    }
    match action {
        NodeAction::Expression(expression) => vec![expression],
        NodeAction::Statement(Stmt::VarDecl(declaration))
        | NodeAction::ForInitializer(ForInitializer::VarDecl(declaration)) => {
            vec![&declaration.initializer]
        }
        NodeAction::Statement(Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. }) => vec![expr],
        NodeAction::Statement(Stmt::Return { expr, .. }) => expr.iter().collect(),
        NodeAction::Statement(Stmt::Throw(statement)) => vec![&statement.expr],
        NodeAction::Statement(Stmt::Increment(increment)) => vec![&increment.target],
        NodeAction::ForIncrement(ForIncrement::Increment(increment)) => vec![&increment.target],
        _ => Vec::new(),
    }
}

fn action_evaluation_exclusions(action: &NodeAction) -> Vec<Span> {
    let expressions = action_expressions(action);
    if expressions.iter().any(|expr| contains_value_region(expr)) {
        expressions.into_iter().map(Expr::span).collect()
    } else {
        let mut closures = Vec::new();
        for expression in expressions {
            collect_closure_exclusions(expression, &mut closures);
        }
        closures
    }
}

fn collect_closure_exclusions(expression: &Expr, spans: &mut Vec<Span>) {
    if matches!(expression, Expr::Closure(_)) {
        spans.push(expression.span());
    } else {
        for child in expression_children(expression) {
            collect_closure_exclusions(child, spans);
        }
    }
}

fn contains_value_region(expression: &Expr) -> bool {
    matches!(expression, Expr::When(_))
        || expression_children(expression)
            .into_iter()
            .any(contains_value_region)
}

/// Executed child expressions in source evaluation order. Closure bodies are
/// separate callable graphs; constructing their environment does not run them.
fn expression_children(expression: &Expr) -> Vec<&Expr> {
    match expression {
        Expr::Grouped { expr, .. }
        | Expr::Unary { expr, .. }
        | Expr::IsType { expr, .. }
        | Expr::PropertyAccess { object: expr, .. } => vec![expr],
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
            ..
        }
        | Expr::ArrayRepeat {
            value: left,
            count: right,
            ..
        }
        | Expr::Index {
            collection: left,
            index: right,
            ..
        } => vec![left, right],
        Expr::FunctionCall { args, .. }
        | Expr::StaticCall { args, .. }
        | Expr::New { args, .. } => args.iter().map(|argument| &argument.value).collect(),
        Expr::MethodCall { object, args, .. }
        | Expr::CallableCall {
            callee: object,
            args,
            ..
        } => std::iter::once(object.as_ref())
            .chain(args.iter().map(|argument| &argument.value))
            .collect(),
        Expr::Array { elements, .. } => elements
            .iter()
            .flat_map(|element| element.key.iter().chain(std::iter::once(&element.value)))
            .collect(),
        Expr::InterpolatedString { parts, .. } => parts
            .iter()
            .filter_map(|part| match part {
                crate::ast::InterpolatedStringPart::Expr(expression) => Some(expression),
                _ => None,
            })
            .collect(),
        Expr::Match {
            scrutinee, arms, ..
        } => std::iter::once(scrutinee.as_ref())
            .chain(arms.iter().flat_map(|arm| {
                arm.guard
                    .iter()
                    .map(|guard| &guard.condition)
                    .chain(std::iter::once(&arm.value))
            }))
            .collect(),
        _ => Vec::new(),
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Block(block) => block.span,
        Stmt::VarDecl(decl) => decl.span,
        Stmt::Assignment(assignment) => assignment.span,
        Stmt::Echo { span, .. } | Stmt::Return { span, .. } | Stmt::Expr { span, .. } => *span,
        Stmt::If(if_stmt) => if_stmt.span,
        Stmt::While(while_stmt) => while_stmt.span,
        Stmt::DoWhile(do_while) => do_while.span,
        Stmt::For(for_stmt) => for_stmt.span,
        Stmt::Break { span } | Stmt::Continue { span } => *span,
        Stmt::Foreach(foreach) => foreach.span,
        Stmt::Increment(increment) => increment.span,
        Stmt::Throw(statement) => statement.span,
        Stmt::Try(statement) => statement.span,
    }
}

fn for_initializer_span(initializer: &ForInitializer) -> Span {
    match initializer {
        ForInitializer::VarDecl(declaration) => declaration.span,
        ForInitializer::Assignment(assignment) => assignment.span,
    }
}

fn for_increment_span(increment: &ForIncrement) -> Span {
    match increment {
        ForIncrement::Increment(increment) => increment.span,
        ForIncrement::Assignment(assignment) => assignment.span,
    }
}

fn deduplicate(mut nodes: Vec<NodeId>) -> Vec<NodeId> {
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}
