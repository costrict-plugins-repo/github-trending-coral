//! # coral-graph
//!
//! Builds call graphs and dependency graphs from parsed symbols produced by coral-index.
//! Provides graph queries (callers, callees, affected files), PageRank scoring,
//! and JSON persistence.

pub mod builder;
pub mod pagerank;
pub mod query;
pub mod store;
pub mod types;

pub use builder::{build_graph, CodeGraph, FileSymbols, ImportInfo, ReferenceInfo, SymbolInfo};
pub use pagerank::pagerank;
pub use store::{load_graph, save_graph};
#[cfg(feature = "bin-format")]
pub use store::{load_graph_bin, save_graph_bin};
pub use types::{EdgeKind, GraphNode, NodeId, NodeKind};
