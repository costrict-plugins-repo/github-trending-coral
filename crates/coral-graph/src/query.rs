use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::builder::CodeGraph;
use crate::pagerank::pagerank;
use crate::types::{EdgeKind, GraphNode, NodeId};

impl CodeGraph {
    /// Find all direct callers of a symbol (nodes with a `Calls` edge pointing to it).
    pub fn callers(&self, id: &NodeId) -> Vec<&GraphNode> {
        let Some(&idx) = self.node_map.get(id) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Incoming)
            .filter(|e| *e.weight() == EdgeKind::Calls)
            .map(|e| &self.graph[e.source()])
            .collect()
    }

    /// Find all symbols called by a function (outgoing `Calls` edges).
    pub fn callees(&self, id: &NodeId) -> Vec<&GraphNode> {
        let Some(&idx) = self.node_map.get(id) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .filter(|e| *e.weight() == EdgeKind::Calls)
            .map(|e| &self.graph[e.target()])
            .collect()
    }

    /// Find all files that depend on a given file (transitively up to `depth`).
    ///
    /// Traverses `DependsOn`, `Imports`, and `Calls` edges in reverse to find
    /// files whose symbols reference symbols in the given file.
    pub fn affected_by(&self, file: &Path, depth: usize) -> Vec<PathBuf> {
        // Find all node indices in the target file
        let file_nodes: HashSet<_> = self
            .node_map
            .iter()
            .filter(|(id, _)| id.file == file)
            .map(|(_, &idx)| idx)
            .collect();

        if file_nodes.is_empty() {
            return Vec::new();
        }

        let mut visited_files: HashSet<PathBuf> = HashSet::new();
        let mut queue: VecDeque<(petgraph::graph::NodeIndex, usize)> = VecDeque::new();

        // Seed the BFS with all nodes in the target file
        for &idx in &file_nodes {
            queue.push_back((idx, 0));
        }

        while let Some((node_idx, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }

            // Look at incoming edges (who depends on this node?)
            for edge in self.graph.edges_directed(node_idx, Direction::Incoming) {
                let kind = *edge.weight();
                if matches!(
                    kind,
                    EdgeKind::Calls | EdgeKind::Imports | EdgeKind::DependsOn | EdgeKind::References
                ) {
                    let source_node = &self.graph[edge.source()];
                    let source_file = &source_node.file;

                    // Skip the file itself
                    if source_file == file {
                        continue;
                    }

                    if visited_files.insert(source_file.clone()) {
                        // Also enqueue all nodes from this newly discovered file
                        for (id, &idx) in &self.node_map {
                            if id.file == *source_file {
                                queue.push_back((idx, current_depth + 1));
                            }
                        }
                    }
                }
            }
        }

        let mut result: Vec<PathBuf> = visited_files.into_iter().collect();
        result.sort();
        result
    }

    /// Find all symbols that reference a given symbol (any incoming edge).
    pub fn references_to(&self, id: &NodeId) -> Vec<&GraphNode> {
        let Some(&idx) = self.node_map.get(id) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|e| &self.graph[e.source()])
            .collect()
    }

    /// PageRank over the graph — most referenced symbols.
    ///
    /// Returns up to `top_n` nodes sorted by descending rank score.
    pub fn hot_symbols(&self, top_n: usize) -> Vec<(&GraphNode, f64)> {
        let scores = pagerank(&self.graph, 0.85, 100);

        let mut indexed: Vec<(usize, f64)> = scores.into_iter().enumerate().collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        indexed.truncate(top_n);

        indexed
            .into_iter()
            .map(|(i, score)| {
                let idx = petgraph::graph::NodeIndex::new(i);
                (&self.graph[idx], score)
            })
            .collect()
    }

    /// Find a symbol by name (case-insensitive substring match).
    pub fn find_symbol(&self, name: &str) -> Vec<&GraphNode> {
        let name_lower = name.to_lowercase();
        self.node_map
            .iter()
            .filter(|(id, _)| id.symbol.to_lowercase().contains(&name_lower))
            .map(|(_, &idx)| &self.graph[idx])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{build_graph, FileSymbols, ReferenceInfo, SymbolInfo};
    use crate::types::NodeKind;

    fn sample_graph() -> CodeGraph {
        let files = vec![
            FileSymbols {
                path: PathBuf::from("src/main.rs"),
                symbols: vec![
                    SymbolInfo {
                        name: "main".to_string(),
                        kind: NodeKind::Function,
                        signature: Some("fn main()".to_string()),
                        line: 1,
                        children: vec![],
                    },
                    SymbolInfo {
                        name: "process".to_string(),
                        kind: NodeKind::Function,
                        signature: Some("fn process()".to_string()),
                        line: 20,
                        children: vec![],
                    },
                ],
                imports: vec![],
                references: vec![
                    ReferenceInfo {
                        from_symbol: "main".to_string(),
                        to_symbol: "process".to_string(),
                        kind: EdgeKind::Calls,
                    },
                    ReferenceInfo {
                        from_symbol: "main".to_string(),
                        to_symbol: "serve".to_string(),
                        kind: EdgeKind::Calls,
                    },
                ],
            },
            FileSymbols {
                path: PathBuf::from("src/server.rs"),
                symbols: vec![
                    SymbolInfo {
                        name: "serve".to_string(),
                        kind: NodeKind::Function,
                        signature: Some("fn serve()".to_string()),
                        line: 1,
                        children: vec![],
                    },
                    SymbolInfo {
                        name: "handle_request".to_string(),
                        kind: NodeKind::Function,
                        signature: Some("fn handle_request()".to_string()),
                        line: 15,
                        children: vec![],
                    },
                ],
                imports: vec![],
                references: vec![ReferenceInfo {
                    from_symbol: "serve".to_string(),
                    to_symbol: "handle_request".to_string(),
                    kind: EdgeKind::Calls,
                }],
            },
        ];

        build_graph(&files)
    }

    #[test]
    fn test_callers_and_callees() {
        let graph = sample_graph();

        let main_id = NodeId {
            file: PathBuf::from("src/main.rs"),
            symbol: "main".to_string(),
        };

        // main calls process and serve
        let callees = graph.callees(&main_id);
        assert_eq!(callees.len(), 2);

        let callee_names: HashSet<&str> = callees.iter().map(|n| n.id.symbol.as_str()).collect();
        assert!(callee_names.contains("process"));
        assert!(callee_names.contains("serve"));

        // process is called by main
        let process_id = NodeId {
            file: PathBuf::from("src/main.rs"),
            symbol: "process".to_string(),
        };
        let callers = graph.callers(&process_id);
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].id.symbol, "main");
    }

    #[test]
    fn test_affected_by() {
        let graph = sample_graph();

        // src/server.rs contains `serve`, which is called by main in src/main.rs
        let affected = graph.affected_by(Path::new("src/server.rs"), 3);
        assert!(affected.contains(&PathBuf::from("src/main.rs")));
    }

    #[test]
    fn test_find_symbol() {
        let graph = sample_graph();

        let results = graph.find_symbol("serve");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.symbol, "serve");

        // Case-insensitive
        let results = graph.find_symbol("MAIN");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.symbol, "main");

        // Substring match
        let results = graph.find_symbol("handle");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.symbol, "handle_request");
    }
}
