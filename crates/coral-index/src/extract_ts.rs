use tree_sitter::Tree;

use crate::extract_helpers::find_child_by_kind_text;
use crate::parser::get_node_text;
use crate::symbol::{Span, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a TypeScript/JavaScript source file's AST.
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
        "type_alias_declaration" => {
            if let Some(sym) = extract_type_alias(node, source) {
                symbols.push(sym);
            }
        }
        "enum_declaration" => {
            if let Some(sym) = extract_enum(node, source) {
                symbols.push(sym);
            }
        }
        "lexical_declaration" => {
            // const/let declarations — might be arrow functions or constants
            extract_lexical_declaration(node, source, symbols);
        }
        "export_statement" => {
            // Unwrap the exported declaration
            let mut inner_cursor = node.walk();
            for child in node.children(&mut inner_cursor) {
                extract_top_level_node(&child, source, symbols);
            }
            // If the last symbol we just added was from the export, mark it public
            // (the child itself doesn't have export visibility)
            // We handle this via checking for the `export` keyword in is_exported
        }
        _ => {}
    }
}

fn extract_function(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "identifier", source)?;
    let doc_comment = extract_jsdoc(node, source);
    let signature = build_function_signature(node, source);
    let visibility = if is_exported(node, source) {
        Visibility::Public
    } else {
        Visibility::Private
    };

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
    let name = find_child_by_kind_text(node, "type_identifier", source)
        .or_else(|| find_child_by_kind_text(node, "identifier", source))?;
    let doc_comment = extract_jsdoc(node, source);
    let visibility = if is_exported(node, source) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let children = extract_class_members(node, source);

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
    let name = find_child_by_kind_text(node, "type_identifier", source)
        .or_else(|| find_child_by_kind_text(node, "identifier", source))?;
    let doc_comment = extract_jsdoc(node, source);
    let visibility = if is_exported(node, source) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let children = extract_interface_members(node, source);

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

fn extract_type_alias(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "type_identifier", source)
        .or_else(|| find_child_by_kind_text(node, "identifier", source))?;
    let doc_comment = extract_jsdoc(node, source);
    let visibility = if is_exported(node, source) {
        Visibility::Public
    } else {
        Visibility::Private
    };
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

fn extract_enum(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_by_kind_text(node, "identifier", source)?;
    let doc_comment = extract_jsdoc(node, source);
    let visibility = if is_exported(node, source) {
        Visibility::Public
    } else {
        Visibility::Private
    };

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

fn extract_lexical_declaration(
    node: &tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = find_child_by_kind_text(&child, "identifier", source);
            if let Some(name) = name {
                // Check if the value is an arrow function or function expression
                let value = child.child_by_field_name("value");
                let kind = match value.as_ref().map(|v| v.kind()) {
                    Some("arrow_function") | Some("function") => SymbolKind::Function,
                    _ => SymbolKind::Const,
                };

                let visibility = if is_exported(node, source) {
                    Visibility::Public
                } else {
                    Visibility::Private
                };

                let signature = if kind == SymbolKind::Function {
                    get_node_text(node, source)
                        .map(|t| t.lines().next().unwrap_or("").trim().to_string())
                } else {
                    get_node_text(node, source)
                        .map(|t| t.trim().trim_end_matches(';').to_string())
                };

                symbols.push(Symbol {
                    name,
                    kind,
                    span: Span::from_node(node),
                    signature,
                    visibility,
                    doc_comment: extract_jsdoc(node, source),
                    children: Vec::new(),
                });
            }
        }
    }
}

fn extract_class_members(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut members = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "class_body" {
            let mut body_cursor = child.walk();
            for member in child.children(&mut body_cursor) {
                match member.kind() {
                    "method_definition" | "public_field_definition" => {
                        let name = find_child_by_kind_text(&member, "property_identifier", source);
                        if let Some(name) = name {
                            let kind = if member.kind() == "method_definition" {
                                SymbolKind::Method
                            } else {
                                SymbolKind::Field
                            };
                            members.push(Symbol {
                                name,
                                kind,
                                span: Span::from_node(&member),
                                signature: get_node_text(&member, source)
                                    .map(|t| t.lines().next().unwrap_or("").trim().to_string()),
                                visibility: Visibility::Public,
                                doc_comment: extract_jsdoc(&member, source),
                                children: Vec::new(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    members
}

fn extract_interface_members(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut members = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "interface_body" || child.kind() == "object_type" {
            let mut body_cursor = child.walk();
            for member in child.children(&mut body_cursor) {
                if member.kind() == "property_signature" || member.kind() == "method_signature" {
                    let name = find_child_by_kind_text(&member, "property_identifier", source);
                    if let Some(name) = name {
                        let kind = if member.kind() == "method_signature" {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Field
                        };
                        members.push(Symbol {
                            name,
                            kind,
                            span: Span::from_node(&member),
                            signature: get_node_text(&member, source)
                                .map(|t| t.trim().trim_end_matches(';').to_string()),
                            visibility: Visibility::Public,
                            doc_comment: None,
                            children: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    members
}

// --- Helpers ---

fn is_exported(node: &tree_sitter::Node, _source: &str) -> bool {
    // Check if the parent is an export_statement
    if let Some(parent) = node.parent()
        && parent.kind() == "export_statement"
    {
        return true;
    }
    false
}

fn extract_jsdoc(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let prev = node.prev_sibling()?;
    if prev.kind() == "comment" {
        let text = get_node_text(&prev, source)?;
        if text.starts_with("/**") {
            // Strip JSDoc markers
            let cleaned: Vec<&str> = text
                .trim_start_matches("/**")
                .trim_end_matches("*/")
                .lines()
                .map(|line| line.trim().trim_start_matches('*').trim())
                .filter(|line| !line.is_empty())
                .collect();
            if cleaned.is_empty() {
                return None;
            }
            return Some(cleaned.join("\n"));
        }
    }
    None
}

fn build_function_signature(node: &tree_sitter::Node, source: &str) -> String {
    let text = get_node_text(node, source).unwrap_or("");
    if let Some(brace_pos) = text.find('{') {
        text[..brace_pos].trim().to_string()
    } else {
        text.lines().next().unwrap_or("").trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::{parse_source, Language};
    use crate::symbol::SymbolKind;
    use std::path::Path;

    #[test]
    fn test_extract_function_declaration() {
        let source = r#"
function greet(name: string): string {
    return `Hello, ${name}!`;
}
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].name, "greet");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_exported_class() {
        let source = r#"
export class UserService {
    getUser(id: string) {
        return null;
    }

    createUser(data: any) {
        return null;
    }
}
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "UserService");
        assert_eq!(sym.kind, SymbolKind::Class);
        // Methods are extracted as children
        assert!(sym.children.len() >= 2);
    }

    #[test]
    fn test_extract_interface() {
        let source = r#"
export interface Config {
    host: string;
    port: number;
    debug: boolean;
}
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        assert_eq!(parsed.symbols.len(), 1);

        let sym = &parsed.symbols[0];
        assert_eq!(sym.name, "Config");
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert_eq!(sym.children.len(), 3);
        assert_eq!(sym.children[0].name, "host");
        assert_eq!(sym.children[0].kind, SymbolKind::Field);
    }

    #[test]
    fn test_extract_arrow_function_const() {
        let source = r#"
const add = (a: number, b: number): number => a + b;
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].name, "add");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_type_alias() {
        let source = r#"
export type UserId = string;
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].name, "UserId");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn test_extract_const_value() {
        let source = r#"
const MAX_RETRIES = 3;
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].name, "MAX_RETRIES");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::Const);
    }

    #[test]
    fn test_extract_enum_ts() {
        let source = r#"
export enum Direction {
    Up = "UP",
    Down = "DOWN",
}
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        assert_eq!(parsed.symbols.len(), 1);
        assert_eq!(parsed.symbols[0].name, "Direction");
        assert_eq!(parsed.symbols[0].kind, SymbolKind::Enum);
    }
}
