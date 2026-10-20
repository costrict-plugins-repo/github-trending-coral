use tree_sitter::Tree;

use crate::extract_helpers::find_child_text;
use crate::parser::get_node_text;
use crate::symbol::{Span, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a Python source file's AST.
pub fn extract_symbols(tree: &Tree, source: &str) -> Vec<Symbol> {
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if let Some(sym) = extract_top_level_node(&child, source) {
            symbols.push(sym);
        }
    }

    symbols
}

fn extract_top_level_node(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    match node.kind() {
        "function_definition" => extract_function(node, source),
        "class_definition" => extract_class(node, source),
        "decorated_definition" => extract_decorated(node, source),
        "expression_statement" => extract_module_assignment(node, source),
        _ => None,
    }
}

fn extract_function(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let doc_comment = extract_docstring(node, source);
    let signature = build_function_signature(node, source);
    let visibility = python_visibility(&name);

    Some(Symbol {
        name,
        kind: SymbolKind::Function,
        span: Span::from_node(node),
        signature: Some(signature),
        visibility,
        doc_comment,
        children: Vec::new(),
    })
}

fn extract_class(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let doc_comment = extract_docstring(node, source);
    let visibility = python_visibility(&name);
    let children = extract_class_body(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Class,
        span: Span::from_node(node),
        signature: None,
        visibility,
        doc_comment,
        children,
    })
}

fn extract_decorated(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    // A decorated_definition wraps a function or class with decorators
    let mut cursor = node.walk();
    let mut decorators = Vec::new();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "decorator" => {
                if let Some(text) = get_node_text(&child, source) {
                    decorators.push(text.to_string());
                }
            }
            "function_definition" => {
                let mut sym = extract_function(&child, source)?;
                // Append decorator info to doc_comment
                if !decorators.is_empty() {
                    let deco_str = decorators.join("\n");
                    sym.doc_comment = Some(match sym.doc_comment {
                        Some(existing) => format!("{deco_str}\n{existing}"),
                        None => deco_str,
                    });
                }
                // Use the outer node span so it includes the decorators
                sym.span = Span::from_node(node);
                return Some(sym);
            }
            "class_definition" => {
                let mut sym = extract_class(&child, source)?;
                if !decorators.is_empty() {
                    let deco_str = decorators.join("\n");
                    sym.doc_comment = Some(match sym.doc_comment {
                        Some(existing) => format!("{deco_str}\n{existing}"),
                        None => deco_str,
                    });
                }
                sym.span = Span::from_node(node);
                return Some(sym);
            }
            _ => {}
        }
    }
    None
}

fn extract_module_assignment(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    // Module-level assignments like `MAX_SIZE = 100` or `__all__ = [...]`
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "assignment" {
            let left = child.child_by_field_name("left")?;
            if left.kind() == "identifier" {
                let name = get_node_text(&left, source)?.to_string();
                // Only extract UPPER_CASE assignments as module constants
                if name.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) {
                    let signature = get_node_text(&child, source)
                        .map(|t| t.trim().to_string());
                    return Some(Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Const,
                        span: Span::from_node(node),
                        signature,
                        visibility: python_visibility(&name),
                        doc_comment: None,
                        children: Vec::new(),
                    });
                }
            }
        }
    }
    None
}

fn extract_class_body(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut members = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "block" {
            let mut block_cursor = child.walk();
            for item in child.children(&mut block_cursor) {
                match item.kind() {
                    "function_definition" => {
                        if let Some(mut sym) = extract_function(&item, source) {
                            sym.kind = SymbolKind::Method;
                            members.push(sym);
                        }
                    }
                    "decorated_definition" => {
                        if let Some(mut sym) = extract_decorated(&item, source) {
                            sym.kind = SymbolKind::Method;
                            members.push(sym);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    members
}

// --- Helpers ---

fn python_visibility(name: &str) -> Visibility {
    if name.starts_with("__") && !name.ends_with("__") {
        // Name-mangled (strongly private)
        Visibility::Private
    } else if name.starts_with('_') {
        // Convention-private
        Visibility::Crate // using Crate to mean "internal/protected"
    } else {
        Visibility::Public
    }
}

fn extract_docstring(node: &tree_sitter::Node, source: &str) -> Option<String> {
    // The docstring is the first expression_statement in the body (block)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "block" {
            let mut block_cursor = child.walk();
            let first_item = child.children(&mut block_cursor).next()?;
            if first_item.kind() == "expression_statement" {
                let mut inner_cursor = first_item.walk();
                for expr in first_item.children(&mut inner_cursor) {
                    if expr.kind() == "string" || expr.kind() == "concatenated_string" {
                        let text = get_node_text(&expr, source)?;
                        return Some(clean_docstring(text));
                    }
                }
            }
        }
    }
    None
}

fn clean_docstring(raw: &str) -> String {
    // Strip triple quotes and clean up
    let stripped = raw
        .trim()
        .trim_start_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("\"\"\"")
        .trim_end_matches("'''");
    stripped
        .lines()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn build_function_signature(node: &tree_sitter::Node, source: &str) -> String {
    let text = get_node_text(node, source).unwrap_or("");
    // Take the `def name(params) -> type:` line
    if let Some(colon_pos) = text.find(':') {
        let first_line = &text[..colon_pos + 1];
        // Only take the first line if colon is on it
        if !first_line.contains('\n') {
            return first_line.trim().to_string();
        }
    }
    text.lines().next().unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use crate::parser::{parse_source, Language};
    use crate::symbol::{SymbolKind, Visibility};
    use std::path::Path;

    #[test]
    fn test_extract_function() {
        let source = r#"
def greet(name: str) -> str:
    """Greet someone by name."""
    return f"Hello, {name}!"
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "greet");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.signature.as_ref().unwrap().contains("def greet"));
        assert_eq!(
            sym.doc_comment.as_deref(),
            Some("Greet someone by name.")
        );
    }

    #[test]
    fn test_extract_class_with_methods() {
        let source = r#"
class UserService:
    """Manages user operations."""

    def __init__(self, db):
        """Initialize the service."""
        self.db = db

    def get_user(self, user_id: str):
        """Retrieve a user by ID."""
        pass

    def _validate(self, data):
        pass
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "UserService");
        assert_eq!(sym.kind, SymbolKind::Class);
        assert_eq!(
            sym.doc_comment.as_deref(),
            Some("Manages user operations.")
        );

        assert_eq!(sym.children.len(), 3);
        assert_eq!(sym.children[0].name, "__init__");
        assert_eq!(sym.children[0].kind, SymbolKind::Method);
        assert_eq!(sym.children[1].name, "get_user");
        assert_eq!(sym.children[2].name, "_validate");
        assert_eq!(sym.children[2].visibility, Visibility::Crate);
    }

    #[test]
    fn test_extract_decorated_function() {
        let source = r#"
@staticmethod
def create(data: dict) -> "User":
    """Create a new user from data."""
    pass
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "create");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.doc_comment.as_ref().unwrap().contains("@staticmethod"));
        assert!(sym.doc_comment.as_ref().unwrap().contains("Create a new user"));
    }

    #[test]
    fn test_extract_module_constant() {
        let source = r#"
MAX_CONNECTIONS = 100
DEFAULT_TIMEOUT = 30
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        assert_eq!(parsed.symbols.len(), 2);
        assert_eq!(parsed.symbols[0].name, "MAX_CONNECTIONS");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::Const);
        assert_eq!(parsed.symbols[1].name, "DEFAULT_TIMEOUT");
    }

    #[test]
    fn test_private_name_convention() {
        let source = r#"
def public_func():
    pass

def _internal_func():
    pass

def __private_func():
    pass
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        assert_eq!(parsed.symbols.len(), 3);
        assert_eq!(parsed.symbols[0].visibility, Visibility::Public);
        assert_eq!(parsed.symbols[1].visibility, Visibility::Crate);
        assert_eq!(parsed.symbols[2].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"
import os
from pathlib import Path
from typing import Optional, List

def main():
    pass
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        assert_eq!(parsed.imports.len(), 3);
        assert!(parsed.imports[0].contains("import os"));
        assert!(parsed.imports[1].contains("from pathlib import Path"));
    }
}
