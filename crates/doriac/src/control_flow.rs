use std::collections::HashMap;

use crate::ast::{Block, ElseBranch, Expr, ForIncrement, ForInitializer, ForStmt, Stmt};
use crate::source::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GivenSemanticInfo {
    pub predicate_statement_indices: Vec<usize>,
}

pub type GivenSemanticInfoMap = HashMap<(usize, usize), GivenSemanticInfo>;

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

struct Builder {
    graph: ControlFlowGraph,
    loops: Vec<LoopContext>,
    finalizers: Vec<Block>,
    given_preludes: GivenSemanticInfoMap,
}

pub fn build_function_cfg(body: &Block, function_span: Span) -> ControlFlowGraph {
    build_function_cfg_with_given(body, function_span, &GivenSemanticInfoMap::new())
}

pub fn build_function_cfg_with_given(
    body: &Block,
    function_span: Span,
    given_preludes: &GivenSemanticInfoMap,
) -> ControlFlowGraph {
    let graph = ControlFlowGraph::new(function_span);
    let entry = graph.entry;
    let mut builder = Builder {
        graph,
        loops: Vec::new(),
        finalizers: Vec::new(),
        given_preludes: given_preludes.clone(),
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

impl Builder {
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
                let value = self.normal(
                    NodeKind::Statement,
                    statement.span,
                    NodeAction::Statement(Stmt::Throw(statement.clone())),
                    incoming,
                );
                let routed = self.route_finalizers(vec![value], 0);
                self.terminal(
                    NodeKind::DivergeExit,
                    statement.span,
                    NodeAction::None,
                    routed,
                );
                Vec::new()
            }
            Stmt::Try(statement) => {
                if let Some(finally) = &statement.finally {
                    self.finalizers.push(finally.body.clone());
                }
                let before = incoming.clone();
                let mut outgoing = self.build_statements(&statement.body.statements, incoming);
                for catch in &statement.catches {
                    outgoing.extend(self.build_statements(&catch.body.statements, before.clone()));
                }
                let outgoing = deduplicate(outgoing);
                if let Some(finally) = &statement.finally {
                    self.finalizers.pop().expect("try finalizer context");
                    self.build_statements(&finally.body.statements, outgoing)
                } else {
                    outgoing
                }
            }
            Stmt::Expr { expr, span } if is_panic_call(expr) => {
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
                    NodeAction::Expression(while_stmt.condition.clone()),
                    true,
                );
                self.graph.connect_all(&gate.passed, header);
                let condition = constant_condition(&while_stmt.condition);
                self.loops.push(LoopContext {
                    continue_target: gate.entry.unwrap_or(header),
                    finalizer_depth: self.finalizers.len(),
                    breaks: Vec::new(),
                });
                let body_incoming = if condition == ConstantCondition::AlwaysFalse {
                    Vec::new()
                } else {
                    vec![self.assumption(&while_stmt.condition, true, header)]
                };
                let body_outgoing =
                    self.build_statements(&while_stmt.body.statements, body_incoming);
                self.graph
                    .connect_all(&body_outgoing, gate.entry.unwrap_or(header));
                let loop_context = self.loops.pop().expect("while loop context");
                let mut outgoing = loop_context.breaks;
                outgoing.extend(gate.failed);
                if condition != ConstantCondition::AlwaysTrue {
                    outgoing.push(self.assumption(&while_stmt.condition, false, header));
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
                    NodeAction::Expression(do_while.condition.clone()),
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
                let condition = constant_condition(&do_while.condition);
                if condition != ConstantCondition::AlwaysFalse {
                    let repeat = self.assumption(&do_while.condition, true, condition_node);
                    self.graph.add_edge(repeat, body_entry);
                }
                let mut outgoing = self.loops.pop().expect("do-while loop context").breaks;
                if condition != ConstantCondition::AlwaysTrue {
                    outgoing.push(self.assumption(&do_while.condition, false, condition_node));
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
                let header = self.graph.add_node(
                    NodeKind::LoopHeader,
                    foreach.iterable.span(),
                    NodeAction::Expression(foreach.iterable.clone()),
                    true,
                );
                self.graph.connect_all(&incoming, header);
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
            .get(&(given.span.start, given.span.end))
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
            let branch = self.graph.add_node(
                NodeKind::Branch,
                predicate.span(),
                NodeAction::Expression(predicate.clone()),
                repeatable,
            );
            self.graph.connect_all(&passed, branch);
            entry.get_or_insert(branch);
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
        let header = self.graph.add_node(
            NodeKind::LoopHeader,
            header_span,
            for_stmt
                .condition
                .clone()
                .map(NodeAction::Expression)
                .unwrap_or(NodeAction::None),
            true,
        );
        self.graph.connect_all(&incoming, header);
        let increment = for_stmt.increment.as_ref().map(|increment| {
            self.graph.add_node(
                NodeKind::Statement,
                for_increment_span(increment),
                NodeAction::ForIncrement(increment.clone()),
                true,
            )
        });
        if let Some(increment) = increment {
            self.graph.add_edge(increment, header);
        }
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
                Some(condition) => vec![self.assumption(condition, true, header)],
                None => vec![header],
            }
        };
        let body_outgoing = self.build_statements(&for_stmt.body.statements, body_incoming);
        self.graph
            .connect_all(&body_outgoing, increment.unwrap_or(header));

        let mut outgoing = self.loops.pop().expect("for loop context").breaks;
        if condition != ConstantCondition::AlwaysTrue {
            if let Some(condition) = &for_stmt.condition {
                outgoing.push(self.assumption(condition, false, header));
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
            incoming = self.build_statements(&active[index].statements, incoming);
        }
        self.finalizers = active;
        incoming
    }

    fn normal(
        &mut self,
        kind: NodeKind,
        span: Span,
        action: NodeAction,
        incoming: Vec<NodeId>,
    ) -> NodeId {
        let node = self
            .graph
            .add_node(kind, span, action, !self.loops.is_empty());
        self.graph.connect_all(&incoming, node);
        node
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
        self.normal(kind, span, action, incoming);
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
