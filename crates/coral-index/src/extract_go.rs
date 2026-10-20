use tree_sitter::Tree;

use crate::extract_helpers::{find_child_by_kind_text, find_child_text};
use crate::parser::get_node_text;
use crate::symbol::{Span, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a Go source file's AST.
pub fn extract_symbols(tree: &Tree, source: &str) -> Vec<Symbol> {
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        extract_top_level_node(&child, source, &mut symbols);
    }

    symbols
}

fn extract_top_level_node(
    node: &tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    match node.kind() {
        "function_declaration" => {
            if let Some(sym) = extract_function(node, source) {
                symbols.push(sym);
            }
        }
        "method_declaration" => {
            if let Some(sym) = extract_method(node, source) {
                symbols.push(sym);
            }
        }
        "type_declaration" => {
            extract_type_declaration(node, source, symbols);
        }
        "const_declaration" => {
            extract_const_declaration(node, source, symbols);
        }
        "var_declaration" => {
            extract_var_declaration(node, source, symbols);
        }
        _ => {}
    }
}

fn extract_function(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);
    let signature = build_function_signature(node, source);

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

fn extract_method(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);
    let signature = build_function_signature(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Method,
        span: Span::from_node(node),
        signature: Some(signature),
        visibility,
        doc_comment,
        children: Vec::new(),
    })
}

fn extract_type_declaration(
    node: &tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec" {
            if let Some(sym) = extract_type_spec(&child, source, node) {
                symbols.push(sym);
            }
        }
    }
}

fn extract_type_spec(
    node: &tree_sitter::Node,
    source: &str,
    parent: &tree_sitter::Node,
) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(parent, source);

    // Determine kind based on the type part
    let mut cursor = node.walk();
    let mut kind = SymbolKind::TypeAlias;
    let mut children = Vec::new();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_type" => {
                kind = SymbolKind::Struct;
                children = extract_struct_fields(&child, source);
            }
            "interface_type" => {
                kind = SymbolKind::Interface;
                children = extract_interface_methods(&child, source);
            }
            _ => {}
        }
    }

    Some(Symbol {
        name,
        kind,
        span: Span::from_node(parent),
        signature: None,
        visibility,
        doc_comment,
        children,
    })
}

fn extract_struct_fields(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut fields = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut inner_cursor = child.walk();
            for item in child.children(&mut inner_cursor) {
                if item.kind() == "field_declaration" {
                    // Field names are in the first identifier(s) before the type
                    let mut field_cursor = item.walk();
                    for field_child in item.children(&mut field_cursor) {
                        if field_child.kind() == "field_identifier" {
                            if let Some(name) = get_node_text(&field_child, source) {
                                let visibility = go_visibility(name);
                                let signature = get_node_text(&item, source)
                                    .map(|t| t.trim().to_string());
                                fields.push(Symbol {
                                    name: name.to_string(),
                                    kind: SymbolKind::Field,
                                    span: Span::from_node(&item),
                                    signature,
                                    visibility,
                                    doc_comment: None,
                                    children: Vec::new(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fields
}

fn extract_interface_methods(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut methods = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "method_elem" || child.kind() == "method_spec" {
            // The method name is a field_identifier child
            if let Some(name) = find_child_by_kind_text(&child, "field_identifier", source) {
                let visibility = go_visibility(&name);
                let signature = get_node_text(&child, source)
                    .map(|t| t.trim().to_string());
                methods.push(Symbol {
                    name,
                    kind: SymbolKind::Method,
                    span: Span::from_node(&child),
                    signature,
                    visibility,
                    doc_comment: None,
                    children: Vec::new(),
                });
            }
        }
    }

    methods
}

fn extract_const_declaration(
    node: &tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_spec" {
            if let Some(name) = find_child_text(&child, "name", source) {
                let visibility = go_visibility(&name);
                let signature = get_node_text(&child, source)
                    .map(|t| t.trim().to_string());
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Const,
                    span: Span::from_node(&child),
                    signature,
                    visibility,
                    doc_comment: None,
                    children: Vec::new(),
                });
            }
        }
    }
}

fn extract_var_declaration(
    node: &tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "var_spec" {
            if let Some(name) = find_child_text(&child, "name", source) {
                let visibility = go_visibility(&name);
                let signature = get_node_text(&child, source)
                    .map(|t| t.trim().to_string());
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Static, // Go package-level vars are analogous to statics
                    span: Span::from_node(&child),
                    signature,
                    visibility,
                    doc_comment: None,
                    children: Vec::new(),
                });
            }
        }
    }
}

// --- Helpers ---

/// In Go, exported identifiers start with an uppercase letter.
fn go_visibility(name: &str) -> Visibility {
    if name.starts_with(|c: char| c.is_uppercase()) {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

fn extract_go_doc_comment(node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Go doc comments are `comment` nodes immediately preceding the declaration
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            if let Some(text) = get_node_text(&sibling, source) {
                let doc = text.trim_start_matches("//").trim();
                comments.push(doc.to_string());
            }
        } else {
            break;
        }
        prev = sibling.prev_sibling();
    }

    if comments.is_empty() {
        return None;
    }

    comments.reverse();
    Some(comments.join("\n"))
}

fn build_function_signature(node: &tree_sitter::Node, source: &str) -> String {
    let text = get_node_text(node, source).unwrap_or("");
    // Take everything up to the opening brace
    if let Some(brace_pos) = text.find('{') {
        text[..brace_pos].trim().to_string()
    } else {
        text.lines().next().unwrap_or("").trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::{parse_source, Language};
    use crate::symbol::{SymbolKind, Visibility};
    use std::path::Path;

    #[test]
    fn test_extract_function() {
        let source = r#"package main

// Greet returns a greeting for the given name.
func Greet(name string) string {
    return "Hello, " + name
}
"#;
        let parsed = parse_source(Path::new("test.go"), source, Language::Go).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Greet");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.signature.as_ref().unwrap().contains("func Greet"));
        assert!(sym
            .doc_comment
            .as_ref()
            .unwrap()
            .contains("Greet returns a greeting"));
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"package main

func helper() bool {
    return true
}
"#;
        let parsed = parse_source(Path::new("test.go"), source, Language::Go).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].name, "helper");
        assert_eq!(parsed.symbols[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_struct() {
        let source = r#"package main

// Config holds application configuration.
type Config struct {
    Name    string
    count   int
    Verbose bool
}
"#;
        let parsed = parse_source(Path::new("test.go"), source, Language::Go).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Config");
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert_eq!(sym.visibility, Visibility::Public);
        assert_eq!(sym.children.len(), 3);
        assert_eq!(sym.children[0].name, "Name");
        assert_eq!(sym.children[0].visibility, Visibility::Public);
        assert_eq!(sym.children[1].name, "count");
        assert_eq!(sym.children[1].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_interface() {
        let source = r#"package main

type Reader interface {
    Read(p []byte) (int, error)
    Close() error
}
"#;
        let parsed = parse_source(Path::new("test.go"), source, Language::Go).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Reader");
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert_eq!(sym.children.len(), 2);
        assert_eq!(sym.children[0].name, "Read");
        assert_eq!(sym.children[0].kind, SymbolKind::Method);
        assert_eq!(sym.children[1].name, "Close");
    }

    #[test]
    fn test_extract_method() {
        let source = r#"package main

func (c *Config) Validate() bool {
    return c.Name != ""
}
"#;
        let parsed = parse_source(Path::new("test.go"), source, Language::Go).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Validate");
        assert_eq!(sym.kind, SymbolKind::Method);
        assert_eq!(sym.visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_const() {
        let source = r#"package main

const MaxSize = 1024
"#;
        let parsed = parse_source(Path::new("test.go"), source, Language::Go).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].name, "MaxSize");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::Const);
        assert_eq!(parsed.symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"package main

import (
    "fmt"
    "os"
)

func main() {
    fmt.Println("hello")
}
"#;
        let parsed = parse_source(Path::new("test.go"), source, Language::Go).unwrap();
        assert_eq!(parsed.imports.len(), 1);
        assert!(parsed.imports[0].contains("fmt"));
    }

}
