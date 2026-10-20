use petgraph::graph::DiGraph;
use petgraph::visit::EdgeRef;

use crate::types::{EdgeKind, GraphNode};

/// Simple iterative PageRank on the code graph.
///
/// Returns scores for each node (higher = more referenced).
/// Uses the standard iterative algorithm:
///   scores[i] = (1-d)/N + d * sum(scores[j]/out_degree[j] for j in incoming[i])
pub fn pagerank(
    graph: &DiGraph<GraphNode, EdgeKind>,
    damping: f64,
    iterations: usize,
) -> Vec<f64> {
    let n = graph.node_count();
    if n == 0 {
        return Vec::new();
    }

    let n_f64 = n as f64;
    let mut scores = vec![1.0 / n_f64; n];
    let mut new_scores = vec![0.0; n];

    // Pre-compute out-degrees for each node
    let out_degrees: Vec<usize> = graph
        .node_indices()
        .map(|idx| graph.edges(idx).count())
        .collect();

    for _ in 0..iterations {
        // Reset new scores with the damping base
        for score in &mut new_scores {
            *score = (1.0 - damping) / n_f64;
        }

        // Distribute rank from each node to its neighbors
        for node_idx in graph.node_indices() {
            let idx = node_idx.index();
            let out_deg = out_degrees[idx];
            if out_deg == 0 {
                // Dangling node: distribute evenly to all nodes
                let contribution = damping * scores[idx] / n_f64;
                for s in &mut new_scores {
                    *s += contribution;
                }
            } else {
                let contribution = damping * scores[idx] / out_deg as f64;
                for edge in graph.edges(node_idx) {
                    let target = edge.target().index();
                    new_scores[target] += contribution;
                }
            }
        }

        std::mem::swap(&mut scores, &mut new_scores);
    }

    scores
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NodeId, NodeKind};
    use petgraph::graph::DiGraph;
    use std::path::PathBuf;

    fn make_node(name: &str) -> GraphNode {
        GraphNode {
            id: NodeId {
                file: PathBuf::from("test.rs"),
                symbol: name.to_string(),
            },
            kind: NodeKind::Function,
            signature: None,
            file: PathBuf::from("test.rs"),
            line: 1,
        }
    }

    #[test]
    fn test_pagerank_empty_graph() {
        let graph: DiGraph<GraphNode, EdgeKind> = DiGraph::new();
        let scores = pagerank(&graph, 0.85, 20);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_pagerank_simple_chain() {
        // A -> B -> C: C should have highest rank
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));
        graph.add_edge(a, b, EdgeKind::Calls);
        graph.add_edge(b, c, EdgeKind::Calls);

        let scores = pagerank(&graph, 0.85, 100);
        assert!(scores[c.index()] > scores[b.index()]);
        assert!(scores[b.index()] > scores[a.index()]);
    }

    #[test]
    fn test_pagerank_converges() {
        // Star topology: A, B, C all point to D
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));
        let d = graph.add_node(make_node("d"));
        graph.add_edge(a, d, EdgeKind::Calls);
        graph.add_edge(b, d, EdgeKind::Calls);
        graph.add_edge(c, d, EdgeKind::Calls);

        let scores = pagerank(&graph, 0.85, 100);

        // D should have the highest score since everyone points to it
        let max_idx = scores
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i);
        assert_eq!(max_idx, Some(d.index()));
    }
}
