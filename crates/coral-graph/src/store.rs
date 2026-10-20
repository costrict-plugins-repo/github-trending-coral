use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::builder::CodeGraph;
use crate::types::{EdgeKind, GraphNode};

/// Serializable representation of the graph for persistence.
#[derive(Serialize, Deserialize)]
struct SerializedGraph {
    nodes: Vec<GraphNode>,
    edges: Vec<(usize, usize, EdgeKind)>, // (from_index, to_index, kind)
}

/// Convert a `CodeGraph` into a serializable representation.
fn to_serializable(graph: &CodeGraph) -> SerializedGraph {
    let nodes: Vec<&GraphNode> = graph
        .graph
        .node_indices()
        .map(|idx| &graph.graph[idx])
        .collect();

    // Build index mapping: NodeIndex -> position in nodes vec
    let index_map: HashMap<NodeIndex, usize> = graph
        .graph
        .node_indices()
        .enumerate()
        .map(|(i, idx)| (idx, i))
        .collect();

    let edges: Vec<(usize, usize, EdgeKind)> = graph
        .graph
        .edge_indices()
        .filter_map(|edge_idx| {
            let (source, target) = graph.graph.edge_endpoints(edge_idx)?;
            let weight = graph.graph[edge_idx];
            Some((index_map[&source], index_map[&target], weight))
        })
        .collect();

    SerializedGraph {
        nodes: nodes.into_iter().cloned().collect(),
        edges,
    }
}

/// Reconstruct a `CodeGraph` from a serializable representation.
///
/// Returns an error if edge indices reference non-existent nodes (corrupted data).
fn from_serializable(sg: SerializedGraph) -> Result<CodeGraph> {
    let mut graph = DiGraph::new();
    let mut node_map = HashMap::new();

    // Add all nodes
    let node_indices: Vec<NodeIndex> = sg
        .nodes
        .iter()
        .map(|node| {
            let idx = graph.add_node(node.clone());
            node_map.insert(node.id.clone(), idx);
            idx
        })
        .collect();

    // Add all edges (with bounds checking for B10)
    for (from_idx, to_idx, kind) in &sg.edges {
        if *from_idx >= node_indices.len() || *to_idx >= node_indices.len() {
            anyhow::bail!("corrupted graph: edge references non-existent node index");
        }
        graph.add_edge(node_indices[*from_idx], node_indices[*to_idx], *kind);
    }

    Ok(CodeGraph { graph, node_map })
}

/// Save a code graph to a JSON file.
pub fn save_graph(graph: &CodeGraph, path: &Path) -> Result<()> {
    let serialized = to_serializable(graph);

    let json = serde_json::to_string_pretty(&serialized)
        .context("Failed to serialize graph to JSON")?;

    std::fs::write(path, json).context("Failed to write graph file")?;

    Ok(())
}

/// Load a code graph from a JSON file.
pub fn load_graph(path: &Path) -> Result<CodeGraph> {
    let json = std::fs::read_to_string(path).context("Failed to read graph file")?;

    let serialized: SerializedGraph =
        serde_json::from_str(&json).context("Failed to deserialize graph from JSON")?;

    from_serializable(serialized)
}

/// Save a code graph to a bincode file (faster, smaller than JSON).
#[cfg(feature = "bin-format")]
pub fn save_graph_bin(graph: &CodeGraph, path: &Path) -> Result<()> {
    let serialized = to_serializable(graph);

    let bytes =
        bincode::serialize(&serialized).context("Failed to serialize graph to bincode")?;

    std::fs::write(path, bytes).context("Failed to write graph bincode file")?;

    Ok(())
}

/// Load a code graph from a bincode file.
#[cfg(feature = "bin-format")]
pub fn load_graph_bin(path: &Path) -> Result<CodeGraph> {
    let bytes = std::fs::read(path).context("Failed to read graph bincode file")?;

    let serialized: SerializedGraph =
        bincode::deserialize(&bytes).context("Failed to deserialize graph from bincode")?;

    from_serializable(serialized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{build_graph, FileSymbols, ReferenceInfo, SymbolInfo};
    use crate::types::{NodeId, NodeKind};
    use std::path::PathBuf;

    #[test]
    fn test_save_and_load_round_trip() {
        let files = vec![FileSymbols {
            path: PathBuf::from("src/main.rs"),
            symbols: vec![
                SymbolInfo {
                    name: "foo".to_string(),
                    kind: NodeKind::Function,
                    signature: Some("fn foo()".to_string()),
                    line: 1,
                    children: vec![],
                },
                SymbolInfo {
                    name: "bar".to_string(),
                    kind: NodeKind::Function,
                    signature: Some("fn bar()".to_string()),
                    line: 10,
                    children: vec![],
                },
            ],
            imports: vec![],
            references: vec![ReferenceInfo {
                from_symbol: "foo".to_string(),
                to_symbol: "bar".to_string(),
                kind: EdgeKind::Calls,
            }],
        }];

        let original = build_graph(&files);

        let dir = std::env::temp_dir();
        let path = dir.join("coral_graph_test.json");

        save_graph(&original, &path).expect("save should succeed");
        let loaded = load_graph(&path).expect("load should succeed");

        assert_eq!(loaded.node_count(), original.node_count());
        assert_eq!(loaded.edge_count(), original.edge_count());

        // Verify node lookup works
        let foo_id = NodeId {
            file: PathBuf::from("src/main.rs"),
            symbol: "foo".to_string(),
        };
        let foo_node = loaded.get_node(&foo_id);
        assert!(foo_node.is_some());
        assert_eq!(foo_node.map(|n| n.line), Some(1));

        // Clean up
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(feature = "bin-format")]
    #[test]
    fn test_bincode_save_and_load_round_trip() {
        let files = vec![FileSymbols {
            path: PathBuf::from("src/main.rs"),
            symbols: vec![
                SymbolInfo {
                    name: "foo".to_string(),
                    kind: NodeKind::Function,
                    signature: Some("fn foo()".to_string()),
                    line: 1,
                    children: vec![],
                },
                SymbolInfo {
                    name: "bar".to_string(),
                    kind: NodeKind::Function,
                    signature: Some("fn bar()".to_string()),
                    line: 10,
                    children: vec![],
                },
            ],
            imports: vec![],
            references: vec![ReferenceInfo {
                from_symbol: "foo".to_string(),
                to_symbol: "bar".to_string(),
                kind: EdgeKind::Calls,
            }],
        }];

        let original = build_graph(&files);

        let dir = std::env::temp_dir();
        let path = dir.join("coral_graph_test.bin");

        save_graph_bin(&original, &path).expect("bincode save should succeed");
        let loaded = load_graph_bin(&path).expect("bincode load should succeed");

        assert_eq!(loaded.node_count(), original.node_count());
        assert_eq!(loaded.edge_count(), original.edge_count());

        // Verify node lookup works
        let foo_id = NodeId {
            file: PathBuf::from("src/main.rs"),
            symbol: "foo".to_string(),
        };
        let foo_node = loaded.get_node(&foo_id);
        assert!(foo_node.is_some());
        assert_eq!(foo_node.map(|n| n.line), Some(1));

        // Verify edge is preserved
        let bar_id = NodeId {
            file: PathBuf::from("src/main.rs"),
            symbol: "bar".to_string(),
        };
        let callees = loaded.callees(&foo_id);
        assert!(callees.iter().any(|n| n.id == bar_id));

        // Clean up
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(feature = "bin-format")]
    #[test]
    fn test_bincode_smaller_than_json() {
        let files = vec![FileSymbols {
            path: PathBuf::from("src/lib.rs"),
            symbols: vec![
                SymbolInfo {
                    name: "alpha".to_string(),
                    kind: NodeKind::Function,
                    signature: Some("fn alpha() -> i32".to_string()),
                    line: 5,
                    children: vec![],
                },
                SymbolInfo {
                    name: "beta".to_string(),
                    kind: NodeKind::Struct,
                    signature: Some("struct Beta".to_string()),
                    line: 20,
                    children: vec![],
                },
            ],
            imports: vec![],
            references: vec![],
        }];

        let graph = build_graph(&files);
        let dir = std::env::temp_dir();

        let json_path = dir.join("coral_size_test.json");
        let bin_path = dir.join("coral_size_test.bin");

        save_graph(&graph, &json_path).expect("json save");
        save_graph_bin(&graph, &bin_path).expect("bincode save");

        let json_size = std::fs::metadata(&json_path).expect("json meta").len();
        let bin_size = std::fs::metadata(&bin_path).expect("bin meta").len();

        assert!(
            bin_size < json_size,
            "bincode ({bin_size}) should be smaller than JSON ({json_size})"
        );

        let _ = std::fs::remove_file(&json_path);
        let _ = std::fs::remove_file(&bin_path);
    }

    #[test]
    fn test_corrupted_graph_edge_bounds() {
        // Manually create a SerializedGraph with an out-of-bounds edge
        let sg = SerializedGraph {
            nodes: vec![GraphNode {
                id: NodeId {
                    file: PathBuf::from("test.rs"),
                    symbol: "foo".to_string(),
                },
                kind: NodeKind::Function,
                signature: None,
                file: PathBuf::from("test.rs"),
                line: 1,
            }],
            edges: vec![(0, 5, EdgeKind::Calls)], // index 5 doesn't exist
        };

        let result = from_serializable(sg);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("corrupted graph"));
    }
}
