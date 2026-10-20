use serde::{Deserialize, Serialize};

/// A symbol extracted from source code (function, struct, class, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
    pub signature: Option<String>,
    pub visibility: Visibility,
    pub doc_comment: Option<String>,
    pub children: Vec<Symbol>,
}

/// The kind of symbol extracted from the AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Const,
    Static,
    TypeAlias,
    Module,
    Method,
    Field,
}

/// A span of source code (line/column based, 0-indexed).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Span {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Visibility of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Crate,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Function => write!(f, "function"),
            Self::Struct => write!(f, "struct"),
            Self::Enum => write!(f, "enum"),
            Self::Trait => write!(f, "trait"),
            Self::Interface => write!(f, "interface"),
            Self::Class => write!(f, "class"),
            Self::Const => write!(f, "const"),
            Self::Static => write!(f, "static"),
            Self::TypeAlias => write!(f, "type alias"),
            Self::Module => write!(f, "module"),
            Self::Method => write!(f, "method"),
            Self::Field => write!(f, "field"),
        }
    }
}

/// A call reference extracted from a function body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallReference {
    pub from_symbol: String, // the function making the call
    pub to_symbol: String,   // the function being called (name only, may be unresolved)
    pub line: u32,
}

impl Span {
    pub fn from_node(node: &tree_sitter::Node) -> Self {
        let start = node.start_position();
        let end = node.end_position();
        Self {
            start_line: start.row as u32,
            start_col: start.column as u32,
            end_line: end.row as u32,
            end_col: end.column as u32,
        }
    }
}
