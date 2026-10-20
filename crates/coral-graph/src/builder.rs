use std::collections::HashMap;
use std::path::PathBuf;

use petgraph::graph::{DiGraph, NodeIndex};
use tracing::debug;

use crate::types::{EdgeKind, GraphNode, NodeId, NodeKind};

/// The central code graph structure, holding nodes (symbols) and edges (relationships).
#[derive(Debug)]
pub struct CodeGraph {
    pub graph: DiGraph<GraphNode, EdgeKind>,
    pub node_map: HashMap<NodeId, NodeIndex>, // fast lookup by ID
}

impl CodeGraph {
    /// Create a new empty code graph.
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_map: HashMap::new(),
        }
    }

    /// Add a node to the graph. If a node with the same ID already exists,
    /// returns the existing index.
    pub fn add_node(&mut self, node: GraphNode) -> NodeIndex {
        if let Some(&idx) = self.node_map.get(&node.id) {
            return idx;
        }
        let id = node.id.clone();
        let idx = self.graph.add_node(node);
        self.node_map.insert(id, idx);
        idx
    }

    /// Add an edge between two nodes identified by their NodeIds.
    /// Returns false if either node is not in the graph.
    pub fn add_edge(&mut self, from: &NodeId, to: &NodeId, kind: EdgeKind) -> bool {
        let from_idx = match self.node_map.get(from) {
            Some(&idx) => idx,
            None => return false,
        };
        let to_idx = match self.node_map.get(to) {
            Some(&idx) => idx,
            None => return false,
        };
        self.graph.add_edge(from_idx, to_idx, kind);
        true
    }

    /// Get a node by its ID.
    pub fn get_node(&self, id: &NodeId) -> Option<&GraphNode> {
        self.node_map.get(id).map(|&idx| &self.graph[idx])
    }

    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsed symbol information from a source file.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: NodeKind,
    pub signature: Option<String>,
    pub line: u32,
    pub children: Vec<SymbolInfo>,
}

/// An import statement from a source file.
#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub from_module: String,
    pub symbols: Vec<String>, // specific imports, or empty for wildcard
}

/// A reference from one symbol to another.
#[derive(Debug, Clone)]
pub struct ReferenceInfo {
    pub from_symbol: String, // the symbol making the reference
    pub to_symbol: String,   // the symbol being referenced
    pub kind: EdgeKind,
}

/// Aggregated symbol data for a single file.
#[derive(Debug, Clone)]
pub struct FileSymbols {
    pub path: PathBuf,
    pub symbols: Vec<SymbolInfo>,
    pub imports: Vec<ImportInfo>,
    pub references: Vec<ReferenceInfo>,
}

/// Build a graph from parsed file data.
///
/// Takes a list of `FileSymbols` structs — each representing a file's parsed
/// symbols, imports, and cross-references — and produces a `CodeGraph`.
pub fn build_graph(files: &[FileSymbols]) -> CodeGraph {
    let mut graph = CodeGraph::new();

    // First pass: add all nodes (symbols) from all files
    for file in files {
        add_symbols_recursive(&mut graph, &file.path, &file.symbols, None);
    }

    // Second pass: collect edges to add, resolving references
    let mut edges_to_add: Vec<(NodeId, NodeId, EdgeKind)> = Vec::new();

    // Build a lookup of symbol name -> NodeId for resolving references
    let symbol_lookup: HashMap<&str, NodeId> = graph
        .node_map
        .keys()
        .map(|id| (id.symbol.as_str(), id.clone()))
        .collect();

    for file in files {
        for reference in &file.references {
            let from_id = NodeId {
                file: file.path.clone(),
                symbol: reference.from_symbol.clone(),
            };
            let to_id = NodeId {
                file: file.path.clone(),
                symbol: reference.to_symbol.clone(),
            };

            // Try exact match first
            if graph.node_map.contains_key(&to_id) {
                edges_to_add.push((from_id, to_id, reference.kind));
            } else if let Some(target_id) = symbol_lookup.get(reference.to_symbol.as_str()) {
                // Fall back to name-based lookup across files
                edges_to_add.push((from_id, target_id.clone(), reference.kind));
            } else {
                debug!(
                    from = %reference.from_symbol,
                    to = %reference.to_symbol,
                    "Unresolved reference"
                );
            }
        }
    }

    // Third pass: add all collected edges
    for (from_id, to_id, kind) in &edges_to_add {
        graph.add_edge(from_id, to_id, *kind);
    }

    graph
}

/// Recursively add symbols and their children to the graph.
fn add_symbols_recursive(
    graph: &mut CodeGraph,
    file: &PathBuf,
    symbols: &[SymbolInfo],
    parent: Option<&NodeId>,
) {
    for sym in symbols {
        let node_id = NodeId {
            file: file.clone(),
            symbol: sym.name.clone(),
        };

        let node = GraphNode {
            id: node_id.clone(),
            kind: sym.kind,
            signature: sym.signature.clone(),
            file: file.clone(),
            line: sym.line,
        };

        graph.add_node(node);

        // If there's a parent, add a Contains edge
        if let Some(parent_id) = parent {
            graph.add_edge(parent_id, &node_id, EdgeKind::Contains);
        }

        // Recurse into children
        if !sym.children.is_empty() {
            add_symbols_recursive(graph, file, &sym.children, Some(&node_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_graph_from_sample_data() {
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
                        name: "helper".to_string(),
                        kind: NodeKind::Function,
                        signature: Some("fn helper() -> i32".to_string()),
                        line: 10,
                        children: vec![],
                    },
                ],
                imports: vec![],
                references: vec![ReferenceInfo {
                    from_symbol: "main".to_string(),
                    to_symbol: "helper".to_string(),
                    kind: EdgeKind::Calls,
                }],
            },
            FileSymbols {
                path: PathBuf::from("src/lib.rs"),
                symbols: vec![SymbolInfo {
                    name: "MyStruct".to_string(),
                    kind: NodeKind::Struct,
                    signature: Some("pub struct MyStruct".to_string()),
                    line: 1,
                    children: vec![SymbolInfo {
                        name: "MyStruct::new".to_string(),
                        kind: NodeKind::Method,
                        signature: Some("pub fn new() -> Self".to_string()),
                        line: 5,
                        children: vec![],
                    }],
                }],
                imports: vec![],
                references: vec![],
            },
        ];

        let graph = build_graph(&files);

        // Should have 4 nodes: main, helper, MyStruct, MyStruct::new
        assert_eq!(graph.node_count(), 4);
        // Should have 2 edges: main->helper (Calls), MyStruct->MyStruct::new (Contains)
        assert_eq!(graph.edge_count(), 2);

        // Verify we can look up nodes
        let main_id = NodeId {
            file: PathBuf::from("src/main.rs"),
            symbol: "main".to_string(),
        };
        let main_node = graph.get_node(&main_id);
        assert!(main_node.is_some());
        assert_eq!(main_node.map(|n| n.line), Some(1));
    }

    #[test]
    fn test_add_duplicate_node() {
        let mut graph = CodeGraph::new();
        let node = GraphNode {
            id: NodeId {
                file: PathBuf::from("test.rs"),
                symbol: "foo".to_string(),
            },
            kind: NodeKind::Function,
            signature: None,
            file: PathBuf::from("test.rs"),
            line: 1,
        };

        let idx1 = graph.add_node(node.clone());
        let idx2 = graph.add_node(node);

        // Should return the same index
        assert_eq!(idx1, idx2);
        assert_eq!(graph.node_count(), 1);
    }
}
