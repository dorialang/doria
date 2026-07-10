use crate::ast::{Block, ElseBranch, FunctionDecl, Stmt};
use crate::control_flow::{build_function_cfg, ControlFlowGraph, Node};
use crate::dataflow::{solve_forward, ForwardAnalysis};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnAnalysis {
    pub graph: ControlFlowGraph,
    pub fallthrough_reachable: bool,
}

pub fn analyze(function: &FunctionDecl) -> ReturnAnalysis {
    analyze_block(&function.body, function.span)
}

pub fn analyze_block(block: &Block, owner_span: crate::source::Span) -> ReturnAnalysis {
    let graph = build_function_cfg(block, owner_span);
    let result = solve_forward(&graph, &Reachability);
    let fallthrough_reachable = result.inputs[graph.fallthrough_exit.0];
    ReturnAnalysis {
        graph,
        fallthrough_reachable,
    }
}

pub fn statement_falls_through(statement: &Stmt) -> bool {
    let span = statement_span(statement);
    analyze_block(
        &Block {
            statements: vec![statement.clone()],
            span,
        },
        span,
    )
    .fallthrough_reachable
}

pub fn block_falls_through(block: &Block) -> bool {
    analyze_block(block, block.span).fallthrough_reachable
}

pub fn else_branch_falls_through(branch: &ElseBranch) -> bool {
    match branch {
        ElseBranch::If(if_stmt) => statement_falls_through(&Stmt::If((**if_stmt).clone())),
        ElseBranch::Block(block) => block_falls_through(block),
    }
}

fn statement_span(statement: &Stmt) -> crate::source::Span {
    match statement {
        Stmt::VarDecl(decl) => decl.span,
        Stmt::Assignment(assignment) => assignment.span,
        Stmt::Echo { span, .. } | Stmt::Return { span, .. } | Stmt::Expr { span, .. } => *span,
        Stmt::If(if_stmt) => if_stmt.span,
        Stmt::While(while_stmt) => while_stmt.span,
        Stmt::For(for_stmt) => for_stmt.span,
        Stmt::Break { span } | Stmt::Continue { span } => *span,
        Stmt::Foreach(foreach) => foreach.span,
        Stmt::Increment(increment) => increment.span,
    }
}

struct Reachability;

impl ForwardAnalysis for Reachability {
    type State = bool;

    fn bottom(&self) -> Self::State {
        false
    }

    fn entry_state(&self) -> Self::State {
        true
    }

    fn transfer(&self, _node: &Node, input: &Self::State) -> Self::State {
        *input
    }

    fn join(&self, state: &mut Self::State, incoming: &Self::State) -> bool {
        let joined = *state || *incoming;
        let changed = joined != *state;
        *state = joined;
        changed
    }
}
