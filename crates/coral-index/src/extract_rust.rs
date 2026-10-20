use tree_sitter::Tree;

use crate::extract_helpers::{find_child_by_kind_text, find_child_text};
use crate::parser::get_node_text;
use crate::symbol::{Span, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a Rust source file's AST.
pub fn extract_symbols(tree: &Tree, source: &str) -> Vec<Symbol> {
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if let Some(sym) = extract_node(&child, source) {
            symbols.push(sym);
        }
    }

    symbols
}

fn extract_node(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    match node.kind() {
        "function_item" => extract_function(node, source),
        "struct_item" => extract_struct(node, source),
        "enum_item" => extract_enum(node, source),
        "trait_item" => extract_trait(node, source),
        "impl_item" => extract_impl(node, source),
        "const_item" => extract_const(node, source),
        "static_item" => extract_static(node, source),
        "type_item" => extract_type_alias(node, source),
        "mod_item" => extract_module(node, source),
        _ => None,
    }
}

fn extract_function(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);
    let signature = extract_function_signature(node, source);

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

fn extract_struct(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "type_identifier", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);
    let children = extract_struct_fields(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Struct,
        span: Span::from_node(node),
        signature: None,
        visibility,
        doc_comment,
        children,
    })
}

fn extract_enum(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "type_identifier", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Enum,
        span: Span::from_node(node),
        signature: None,
        visibility,
        doc_comment,
        children: Vec::new(),
    })
}

fn extract_trait(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "type_identifier", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);
    let children = extract_trait_methods(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Trait,
        span: Span::from_node(node),
        signature: None,
        visibility,
        doc_comment,
        children,
    })
}

fn extract_impl(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    // Find the type being implemented
    let name = find_impl_target(node, source)?;
    let children = extract_impl_methods(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Struct, // impl blocks map to the struct/type
        span: Span::from_node(node),
        signature: get_node_text(node, source)
            .map(|t| t.lines().next().unwrap_or("").to_string()),
        visibility: Visibility::Public,
        doc_comment: None,
        children,
    })
}

fn extract_const(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);
    let signature = get_node_text(node, source)
        .map(|t| t.lines().next().unwrap_or("").trim().to_string());

    Some(Symbol {
        name,
        kind: SymbolKind::Const,
        span: Span::from_node(node),
        signature,
        visibility,
        doc_comment,
        children: Vec::new(),
    })
}

fn extract_static(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);
    let signature = get_node_text(node, source)
        .map(|t| t.lines().next().unwrap_or("").trim().to_string());

    Some(Symbol {
        name,
        kind: SymbolKind::Static,
        span: Span::from_node(node),
        signature,
        visibility,
        doc_comment,
        children: Vec::new(),
    })
}

fn extract_type_alias(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "type_identifier", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);
    let signature = get_node_text(node, source).map(|t| t.trim().to_string());

    Some(Symbol {
        name,
        kind: SymbolKind::TypeAlias,
        span: Span::from_node(node),
        signature,
        visibility,
        doc_comment,
        children: Vec::new(),
    })
}

fn extract_module(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "identifier", source)?;
    let visibility = extract_visibility(node, source);
    let doc_comment = extract_doc_comment(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Module,
        span: Span::from_node(node),
        signature: None,
        visibility,
        doc_comment,
        children: Vec::new(),
    })
}

// --- Helpers ---

fn extract_visibility(node: &tree_sitter::Node, source: &str) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = get_node_text(&child, source).unwrap_or("");
            if text.contains("crate") {
                return Visibility::Crate;
            }
            return Visibility::Public;
        }
    }
    Visibility::Private
}

fn extract_doc_comment(node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Doc comments appear as preceding siblings with kind "line_comment" starting with "///"
    // or "block_comment" starting with "/**"
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "line_comment" {
            if let Some(text) = get_node_text(&sibling, source) {
                if text.starts_with("///") {
                    let doc = text.trim_start_matches("///").trim();
                    comments.push(doc.to_string());
                } else {
                    break;
                }
            }
        } else if sibling.kind() == "attribute_item" {
            // Skip #[...] attributes between doc comments and the item
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

fn extract_function_signature(node: &tree_sitter::Node, source: &str) -> String {
    // Build signature from the first line up to the opening brace
    let text = get_node_text(node, source).unwrap_or("");
    if let Some(brace_pos) = text.find('{') {
        text[..brace_pos].trim().to_string()
    } else {
        // No body (e.g., trait method declaration with semicolon)
        text.lines().next().unwrap_or("").trim().to_string()
    }
}

fn find_impl_target(node: &tree_sitter::Node, source: &str) -> Option<String> {
    // For `impl Foo` -> type is the first type_identifier after `impl`
    // For `impl Trait for Foo` -> type is after `for`
    node.child_by_field_name("type")
        .and_then(|n| {
            // The type field might be a type_identifier or a generic_type
            if n.kind() == "type_identifier" {
                get_node_text(&n, source).map(|s| s.to_string())
            } else if n.kind() == "generic_type" {
                // Get the first type_identifier child
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if child.kind() == "type_identifier" {
                        return get_node_text(&child, source).map(|s| s.to_string());
                    }
                }
                get_node_text(&n, source).map(|s| s.to_string())
            } else {
                get_node_text(&n, source).map(|s| s.to_string())
            }
        })
}

fn extract_impl_methods(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut methods = Vec::new();
    let mut cursor = node.walk();

    // Look for the declaration_list (body of impl)
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            let mut inner_cursor = child.walk();
            for item in child.children(&mut inner_cursor) {
                if item.kind() == "function_item"
                    && let Some(mut sym) = extract_function(&item, source)
                {
                    sym.kind = SymbolKind::Method;
                    methods.push(sym);
                }
            }
        }
    }

    methods
}

fn extract_trait_methods(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut methods = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            let mut inner_cursor = child.walk();
            for item in child.children(&mut inner_cursor) {
                if item.kind() == "function_signature_item" || item.kind() == "function_item" {
                    let name = find_child_text(&item, "name", source);
                    if let Some(name) = name {
                        let signature = extract_function_signature(&item, source);
                        methods.push(Symbol {
                            name,
                            kind: SymbolKind::Method,
                            span: Span::from_node(&item),
                            signature: Some(signature),
                            visibility: Visibility::Public,
                            doc_comment: extract_doc_comment(&item, source),
                            children: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    methods
}

fn extract_struct_fields(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut fields = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut inner_cursor = child.walk();
            for item in child.children(&mut inner_cursor) {
                if item.kind() == "field_declaration" {
                    let name = find_child_text(&item, "name", source);
                    if let Some(name) = name {
                        let visibility = extract_visibility(&item, source);
                        fields.push(Symbol {
                            name,
                            kind: SymbolKind::Field,
                            span: Span::from_node(&item),
                            signature: get_node_text(&item, source)
                                .map(|t| t.trim().trim_end_matches(',').to_string()),
                            visibility,
                            doc_comment: None,
                            children: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{parse_source, Language};
    use std::path::Path;

    #[test]
    fn test_extract_function() {
        let source = r#"
/// Adds two numbers together.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "add");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.signature.as_ref().unwrap().contains("pub fn add"));
        assert_eq!(
            sym.doc_comment.as_deref(),
            Some("Adds two numbers together.")
        );
    }

    #[test]
    fn test_extract_struct_with_fields() {
        let source = r#"
pub struct Config {
    pub name: String,
    pub count: usize,
    secret: Vec<u8>,
}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Config");
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert_eq!(sym.visibility, Visibility::Public);
        assert_eq!(sym.children.len(), 3);

        assert_eq!(sym.children[0].name, "name");
        assert_eq!(sym.children[0].visibility, Visibility::Public);
        assert_eq!(sym.children[1].name, "count");
        assert_eq!(sym.children[2].name, "secret");
        assert_eq!(sym.children[2].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_enum() {
        let source = r#"
/// Possible directions.
pub enum Direction {
    North,
    South,
    East,
    West,
}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Direction");
        assert_eq!(sym.kind, SymbolKind::Enum);
        assert_eq!(sym.visibility, Visibility::Public);
        assert_eq!(
            sym.doc_comment.as_deref(),
            Some("Possible directions.")
        );
    }

    #[test]
    fn test_extract_trait_with_methods() {
        let source = r#"
pub trait Drawable {
    fn draw(&self);
    fn resize(&mut self, width: u32, height: u32);
}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Drawable");
        assert_eq!(sym.kind, SymbolKind::Trait);
        assert_eq!(sym.children.len(), 2);
        assert_eq!(sym.children[0].name, "draw");
        assert_eq!(sym.children[0].kind, SymbolKind::Method);
        assert_eq!(sym.children[1].name, "resize");
    }

    #[test]
    fn test_extract_impl_block() {
        let source = r#"
impl Config {
    pub fn new() -> Self {
        Self { name: String::new(), count: 0, secret: vec![] }
    }

    fn validate(&self) -> bool {
        !self.name.is_empty()
    }
}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Config");
        assert_eq!(sym.children.len(), 2);
        assert_eq!(sym.children[0].name, "new");
        assert_eq!(sym.children[0].kind, SymbolKind::Method);
        assert_eq!(sym.children[0].visibility, Visibility::Public);
        assert_eq!(sym.children[1].name, "validate");
        assert_eq!(sym.children[1].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_const_and_type_alias() {
        let source = r#"
pub const MAX_SIZE: usize = 1024;

pub type Result<T> = std::result::Result<T, Error>;
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert_eq!(parsed.symbols.len(), 2);

        assert_eq!(parsed.symbols[0].name, "MAX_SIZE");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::Const);
        assert_eq!(parsed.symbols[0].visibility, Visibility::Public);

        assert_eq!(parsed.symbols[1].name, "Result");
        assert_eq!(parsed.symbols[1].kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"
fn helper() -> bool {
    true
}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].visibility, Visibility::Private);
    }
}
