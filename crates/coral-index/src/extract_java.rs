use tree_sitter::Tree;

use crate::extract_helpers::find_child_text;
use crate::parser::get_node_text;
use crate::symbol::{Span, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a Java source file's AST.
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
        "class_declaration" => {
            if let Some(sym) = extract_class(node, source) {
                symbols.push(sym);
            }
        }
        "interface_declaration" => {
            if let Some(sym) = extract_interface(node, source) {
                symbols.push(sym);
            }
        }
        "enum_declaration" => {
            if let Some(sym) = extract_enum(node, source) {
                symbols.push(sym);
            }
        }
        _ => {}
    }
}

fn extract_class(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_java_visibility(node, source);
    let doc_comment = extract_javadoc(node, source);
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

fn extract_interface(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_java_visibility(node, source);
    let doc_comment = extract_javadoc(node, source);
    let children = extract_interface_body(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Interface,
        span: Span::from_node(node),
        signature: None,
        visibility,
        doc_comment,
        children,
    })
}

fn extract_enum(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_java_visibility(node, source);
    let doc_comment = extract_javadoc(node, source);

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

fn extract_class_body(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut members = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "class_body" {
            let mut body_cursor = child.walk();
            for item in child.children(&mut body_cursor) {
                match item.kind() {
                    "method_declaration" => {
                        if let Some(sym) = extract_method(&item, source) {
                            members.push(sym);
                        }
                    }
                    "constructor_declaration" => {
                        if let Some(sym) = extract_constructor(&item, source) {
                            members.push(sym);
                        }
                    }
                    "field_declaration" => {
                        extract_field_declaration(&item, source, &mut members);
                    }
                    "class_declaration" => {
                        // Inner class
                        if let Some(sym) = extract_class(&item, source) {
                            members.push(sym);
                        }
                    }
                    "interface_declaration" => {
                        if let Some(sym) = extract_interface(&item, source) {
                            members.push(sym);
                        }
                    }
                    "enum_declaration" => {
                        if let Some(sym) = extract_enum(&item, source) {
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

fn extract_interface_body(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut members = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "interface_body" {
            let mut body_cursor = child.walk();
            for item in child.children(&mut body_cursor) {
                if item.kind() == "method_declaration" {
                    if let Some(sym) = extract_method(&item, source) {
                        members.push(sym);
                    }
                }
            }
        }
    }

    members
}

fn extract_method(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_java_visibility(node, source);
    let doc_comment = extract_javadoc(node, source);
    let signature = build_method_signature(node, source);

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

fn extract_constructor(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let visibility = extract_java_visibility(node, source);
    let doc_comment = extract_javadoc(node, source);
    let signature = build_method_signature(node, source);

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

fn extract_field_declaration(
    node: &tree_sitter::Node,
    source: &str,
    members: &mut Vec<Symbol>,
) {
    let visibility = extract_java_visibility(node, source);
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name) = find_child_text(&child, "name", source) {
                let signature = get_node_text(node, source)
                    .map(|t| t.trim().trim_end_matches(';').to_string());
                members.push(Symbol {
                    name,
                    kind: SymbolKind::Field,
                    span: Span::from_node(node),
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

fn extract_java_visibility(node: &tree_sitter::Node, source: &str) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = get_node_text(&child, source).unwrap_or("");
            if text.contains("public") {
                return Visibility::Public;
            } else if text.contains("protected") {
                return Visibility::Crate; // using Crate for package/protected visibility
            } else if text.contains("private") {
                return Visibility::Private;
            }
        }
    }
    // Java default (package-private)
    Visibility::Crate
}

fn extract_javadoc(node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Javadoc comments appear as preceding siblings
    let prev = node.prev_sibling()?;
    if prev.kind() == "block_comment" || prev.kind() == "line_comment" {
        let text = get_node_text(&prev, source)?;
        if text.starts_with("/**") {
            return Some(clean_javadoc(text));
        } else if text.starts_with("//") {
            // Single-line doc comment
            let doc = text.trim_start_matches("//").trim();
            return Some(doc.to_string());
        }
    }
    None
}

fn clean_javadoc(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("/**")
        .trim_end_matches("*/")
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_method_signature(node: &tree_sitter::Node, source: &str) -> String {
    let text = get_node_text(node, source).unwrap_or("");
    // Take everything up to the opening brace
    if let Some(brace_pos) = text.find('{') {
        text[..brace_pos].trim().to_string()
    } else {
        // Abstract method (ends with semicolon)
        text.lines().next().unwrap_or("").trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::{parse_source, Language};
    use crate::symbol::{SymbolKind, Visibility};
    use std::path::Path;

    #[test]
    fn test_extract_class() {
        let source = r#"
public class UserService {
    private String name;
    public int count;

    public UserService(String name) {
        this.name = name;
    }

    public String getName() {
        return this.name;
    }

    private void validate() {
        // internal
    }
}
"#;
        let parsed = parse_source(Path::new("Test.java"), source, Language::Java).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "UserService");
        assert_eq!(sym.kind, SymbolKind::Class);
        assert_eq!(sym.visibility, Visibility::Public);

        // Should have fields and methods
        let methods: Vec<_> = sym.children.iter().filter(|c| c.kind == SymbolKind::Method).collect();
        let fields: Vec<_> = sym.children.iter().filter(|c| c.kind == SymbolKind::Field).collect();

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "name");
        assert_eq!(fields[0].visibility, Visibility::Private);
        assert_eq!(fields[1].name, "count");
        assert_eq!(fields[1].visibility, Visibility::Public);

        assert!(methods.len() >= 2);
        assert!(methods.iter().any(|m| m.name == "getName"));
        assert!(methods.iter().any(|m| m.name == "validate"));
    }

    #[test]
    fn test_extract_interface() {
        let source = r#"
public interface Repository {
    void save(Object entity);
    Object findById(String id);
}
"#;
        let parsed = parse_source(Path::new("Test.java"), source, Language::Java).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Repository");
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert_eq!(sym.visibility, Visibility::Public);
        assert_eq!(sym.children.len(), 2);
        assert_eq!(sym.children[0].name, "save");
        assert_eq!(sym.children[0].kind, SymbolKind::Method);
        assert_eq!(sym.children[1].name, "findById");
    }

    #[test]
    fn test_extract_enum() {
        let source = r#"
public enum Direction {
    NORTH,
    SOUTH,
    EAST,
    WEST
}
"#;
        let parsed = parse_source(Path::new("Test.java"), source, Language::Java).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Direction");
        assert_eq!(sym.kind, SymbolKind::Enum);
        assert_eq!(sym.visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"
import java.util.List;
import java.io.IOException;

public class App {
    public void run() {}
}
"#;
        let parsed = parse_source(Path::new("Test.java"), source, Language::Java).unwrap();
        assert_eq!(parsed.imports.len(), 2);
        assert!(parsed.imports[0].contains("java.util.List"));
        assert!(parsed.imports[1].contains("java.io.IOException"));
    }

    #[test]
    fn test_visibility_modifiers() {
        let source = r#"
public class Example {
    public void publicMethod() {}
    private void privateMethod() {}
    protected void protectedMethod() {}
    void packageMethod() {}
}
"#;
        let parsed = parse_source(Path::new("Test.java"), source, Language::Java).unwrap();
        let sym = &parsed.symbols[0];

        let methods: Vec<_> = sym.children.iter().filter(|c| c.kind == SymbolKind::Method).collect();
        assert_eq!(methods.len(), 4);

        let public_method = methods.iter().find(|m| m.name == "publicMethod").unwrap();
        assert_eq!(public_method.visibility, Visibility::Public);

        let private_method = methods.iter().find(|m| m.name == "privateMethod").unwrap();
        assert_eq!(private_method.visibility, Visibility::Private);

        let protected_method = methods.iter().find(|m| m.name == "protectedMethod").unwrap();
        assert_eq!(protected_method.visibility, Visibility::Crate);

        let package_method = methods.iter().find(|m| m.name == "packageMethod").unwrap();
        assert_eq!(package_method.visibility, Visibility::Crate);
    }
}
