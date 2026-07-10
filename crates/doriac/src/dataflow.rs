use std::collections::BTreeSet;

use crate::control_flow::{ControlFlowGraph, Node, NodeId};

pub trait ForwardAnalysis {
    type State: Clone + Eq;

    fn bottom(&self) -> Self::State;
    fn entry_state(&self) -> Self::State;
    fn transfer(&self, node: &Node, input: &Self::State) -> Self::State;
    fn join(&self, state: &mut Self::State, incoming: &Self::State) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataflowResult<State> {
    pub inputs: Vec<State>,
    pub outputs: Vec<State>,
    pub traversal_order: Vec<NodeId>,
}

pub fn solve_forward<A>(graph: &ControlFlowGraph, analysis: &A) -> DataflowResult<A::State>
where
    A: ForwardAnalysis,
{
    let mut inputs = vec![analysis.bottom(); graph.nodes.len()];
    let mut outputs = vec![analysis.bottom(); graph.nodes.len()];
    inputs[graph.entry.0] = analysis.entry_state();

    let mut pending = BTreeSet::from([graph.entry]);
    let mut traversal_order = Vec::new();
    while let Some(node_id) = pending.pop_first() {
        traversal_order.push(node_id);
        let node = &graph.nodes[node_id.0];
        let output = analysis.transfer(node, &inputs[node_id.0]);
        if outputs[node_id.0] != output {
            outputs[node_id.0] = output.clone();
        }

        for successor in &node.successors {
            if analysis.join(&mut inputs[successor.0], &output) {
                pending.insert(*successor);
            }
        }
    }

    DataflowResult {
        inputs,
        outputs,
        traversal_order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_flow::{ControlFlowGraph, NodeKind};
    use crate::source::Span;

    #[derive(Clone, Copy)]
    struct MaximumPath;

    impl ForwardAnalysis for MaximumPath {
        type State = usize;

        fn bottom(&self) -> Self::State {
            0
        }

        fn entry_state(&self) -> Self::State {
            1
        }

        fn transfer(&self, _node: &Node, input: &Self::State) -> Self::State {
            *input + 1
        }

        fn join(&self, state: &mut Self::State, incoming: &Self::State) -> bool {
            if *incoming <= *state {
                return false;
            }
            *state = *incoming;
            true
        }
    }

    #[test]
    fn generic_join_and_traversal_are_deterministic() {
        let mut graph = ControlFlowGraph::new_for_test(NodeKind::Entry, Span::new(0, 1));
        let left = graph.add_node_for_test(NodeKind::Statement, Span::new(1, 2));
        let right = graph.add_node_for_test(NodeKind::Statement, Span::new(2, 3));
        let exit = graph.add_node_for_test(NodeKind::FallthroughExit, Span::new(3, 4));
        graph.add_edge_for_test(graph.entry, right);
        graph.add_edge_for_test(graph.entry, left);
        graph.add_edge_for_test(left, exit);
        graph.add_edge_for_test(right, exit);

        let first = solve_forward(&graph, &MaximumPath);
        let second = solve_forward(&graph, &MaximumPath);

        assert_eq!(first, second);
        assert_eq!(first.inputs[exit.0], 3);
        assert_eq!(first.traversal_order[0], graph.entry);
    }
}
