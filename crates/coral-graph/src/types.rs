use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Unique identifier for a node in the code graph.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeId {
    pub file: PathBuf,
    pub symbol: String, // fully qualified name
}

/// A node representing a code symbol in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub signature: Option<String>,
    pub file: PathBuf,
    pub line: u32,
}

/// The kind of symbol a node represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Module,
    File,
    Const,
    TypeAlias,
}

/// The kind of relationship between two nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Function A calls function B
    Calls,
    /// Module A imports from module B
    Imports,
    /// Type A implements trait/interface B
    Implements,
    /// Class A extends class B
    Extends,
    /// Symbol A references symbol B
    References,
    /// Module A contains symbol B
    Contains,
    /// File A depends on file B
    DependsOn,
}
