use tree_sitter::Tree;

use crate::extract_helpers::find_child_text;
use crate::parser::get_node_text;
use crate::symbol::{Span, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a C source file's AST.
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
        "function_definition" => {
            if let Some(sym) = extract_function(node, source) {
                symbols.push(sym);
            }
        }
        "declaration" => {
            extract_declaration(node, source, symbols);
        }
        "type_definition" => {
            if let Some(sym) = extract_typedef(node, source) {
                symbols.push(sym);
            }
        }
        "preproc_function_def" => {
            if let Some(sym) = extract_macro(node, source) {
                symbols.push(sym);
            }
        }
        "preproc_def" => {
            if let Some(sym) = extract_macro(node, source) {
                symbols.push(sym);
            }
        }
        _ => {}
    }
}

fn extract_function(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = extract_declarator_name(&declarator, source)?;
    let visibility = c_visibility(node, source);
    let doc_comment = extract_c_doc_comment(node, source);
    let signature = build_c_function_signature(node, source);

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

/// Extract the name from a declarator node, handling nested declarators
/// (e.g., `function_declarator` wrapping an `identifier`).
fn extract_declarator_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => get_node_text(node, source).map(|s| s.to_string()),
        "function_declarator" | "pointer_declarator" | "array_declarator"
        | "parenthesized_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_declarator_name(&inner, source)
        }
        _ => {
            // Try the "declarator" field first, then the "name" field
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_declarator_name(&inner, source);
            }
            find_child_text(node, "name", source)
        }
    }
}

fn extract_declaration(
    node: &tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    // A top-level `declaration` can be a global variable or a function prototype.
    // We detect function prototypes by the presence of a `function_declarator` child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_declarator" {
            // This is a function prototype — skip (we care about definitions)
            return;
        }
        if child.kind() == "init_declarator" || child.kind() == "identifier" {
            // This is a variable declaration
            let name = if child.kind() == "init_declarator" {
                child
                    .child_by_field_name("declarator")
                    .and_then(|d| extract_declarator_name(&d, source))
            } else {
                get_node_text(&child, source).map(|s| s.to_string())
            };

            if let Some(name) = name {
                let visibility = c_visibility(node, source);
                let signature = get_node_text(node, source)
                    .map(|t| t.trim().trim_end_matches(';').to_string());
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Static, // file-scope globals are analogous to statics
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

fn extract_typedef(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let doc_comment = extract_c_doc_comment(node, source);

    // Check for struct/enum inside the typedef
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_specifier" => {
                // typedef struct { ... } Name;
                let name = extract_typedef_name(node, source)?;
                let fields = extract_struct_fields(&child, source);
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Struct,
                    span: Span::from_node(node),
                    signature: None,
                    visibility: Visibility::Public,
                    doc_comment,
                    children: fields,
                });
            }
            "enum_specifier" => {
                // typedef enum { ... } Name;
                let name = extract_typedef_name(node, source)?;
                let variants = extract_enum_variants(&child, source);
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Enum,
                    span: Span::from_node(node),
                    signature: None,
                    visibility: Visibility::Public,
                    doc_comment,
                    children: variants,
                });
            }
            _ => {}
        }
    }

    // Plain typedef alias (e.g., `typedef int myint;`)
    let name = extract_typedef_name(node, source)?;
    Some(Symbol {
        name,
        kind: SymbolKind::TypeAlias,
        span: Span::from_node(node),
        signature: get_node_text(node, source).map(|t| t.trim().to_string()),
        visibility: Visibility::Public,
        doc_comment,
        children: Vec::new(),
    })
}

/// Extract the typedef'd name — it's the last `type_identifier` child of the `type_definition`.
fn extract_typedef_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    // The typedef name is a `type_identifier` child (not field-named, so we iterate).
    let mut cursor = node.walk();
    let mut last_type_id = None;
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            last_type_id = get_node_text(&child, source).map(|s| s.to_string());
        }
    }
    last_type_id
}

fn extract_struct_fields(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut fields = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut inner_cursor = child.walk();
            for item in child.children(&mut inner_cursor) {
                if item.kind() == "field_declaration" {
                    // Find the field_identifier child(ren)
                    let mut field_cursor = item.walk();
                    for field_child in item.children(&mut field_cursor) {
                        if field_child.kind() == "field_identifier" {
                            if let Some(name) = get_node_text(&field_child, source) {
                                let signature = get_node_text(&item, source)
                                    .map(|t| t.trim().trim_end_matches(';').to_string());
                                fields.push(Symbol {
                                    name: name.to_string(),
                                    kind: SymbolKind::Field,
                                    span: Span::from_node(&item),
                                    signature,
                                    visibility: Visibility::Public,
                                    doc_comment: None,
                                    children: Vec::new(),
                                });
                            }
                        }
                        // Also handle declarators that wrap field_identifiers
                        if field_child.kind() == "pointer_declarator"
                            || field_child.kind() == "array_declarator"
                        {
                            if let Some(name) = extract_field_declarator_name(&field_child, source) {
                                let signature = get_node_text(&item, source)
                                    .map(|t| t.trim().trim_end_matches(';').to_string());
                                fields.push(Symbol {
                                    name,
                                    kind: SymbolKind::Field,
                                    span: Span::from_node(&item),
                                    signature,
                                    visibility: Visibility::Public,
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

fn extract_field_declarator_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "field_identifier" => get_node_text(node, source).map(|s| s.to_string()),
        "pointer_declarator" | "array_declarator" | "parenthesized_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_field_declarator_name(&inner, source)
        }
        _ => None,
    }
}

fn extract_enum_variants(node: &tree_sitter::Node, source: &str) -> Vec<Symbol> {
    let mut variants = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "enumerator_list" {
            let mut inner_cursor = child.walk();
            for item in child.children(&mut inner_cursor) {
                if item.kind() == "enumerator" {
                    if let Some(name) = find_child_text(&item, "name", source) {
                        let signature = get_node_text(&item, source)
                            .map(|t| t.trim().to_string());
                        variants.push(Symbol {
                            name,
                            kind: SymbolKind::Const, // enum variants are constants
                            span: Span::from_node(&item),
                            signature,
                            visibility: Visibility::Public,
                            doc_comment: None,
                            children: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    variants
}

fn extract_macro(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let doc_comment = extract_c_doc_comment(node, source);
    let signature = get_node_text(node, source).map(|t| {
        // Take first line only for macros
        t.lines().next().unwrap_or("").trim().to_string()
    });

    Some(Symbol {
        name,
        kind: SymbolKind::Const, // macros are constant-like
        span: Span::from_node(node),
        signature,
        visibility: Visibility::Public,
        doc_comment,
        children: Vec::new(),
    })
}

// --- Helpers ---

/// In C, `static` at file scope means internal linkage (private).
/// Everything else is public by default.
fn c_visibility(node: &tree_sitter::Node, source: &str) -> Visibility {
    // Check for storage_class_specifier with "static"
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "storage_class_specifier" {
            if let Some(text) = get_node_text(&child, source) {
                if text == "static" {
                    return Visibility::Private;
                }
            }
        }
    }
    Visibility::Public
}

fn extract_c_doc_comment(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            if let Some(text) = get_node_text(&sibling, source) {
                let doc = if text.starts_with("/*") {
                    // Block comment: strip /* and */
                    text.trim()
                        .trim_start_matches("/*")
                        .trim_end_matches("*/")
                        .lines()
                        .map(|line| line.trim().trim_start_matches('*').trim())
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    // Line comment: strip //
                    text.trim_start_matches("//").trim().to_string()
                };
                if !doc.is_empty() {
                    comments.push(doc);
                }
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

fn build_c_function_signature(node: &tree_sitter::Node, source: &str) -> String {
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
        let source = r#"
/* Compute the sum of two integers. */
int add(int a, int b) {
    return a + b;
}
"#;
        let parsed = parse_source(Path::new("test.c"), source, Language::C).unwrap();
        let funcs: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);

        let sym = funcs[0];
        assert_eq!(sym.name, "add");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.signature.as_ref().unwrap().contains("int add"));
        assert!(sym
            .doc_comment
            .as_ref()
            .unwrap()
            .contains("Compute the sum"));
    }

    #[test]
    fn test_extract_static_function() {
        let source = r#"
static void helper(void) {
    // internal
}
"#;
        let parsed = parse_source(Path::new("test.c"), source, Language::C).unwrap();
        let funcs: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "helper");
        assert_eq!(funcs[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_typedef_struct() {
        let source = r#"
typedef struct {
    int x;
    int y;
} Point;
"#;
        let parsed = parse_source(Path::new("test.c"), source, Language::C).unwrap();
        let structs: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert_eq!(structs.len(), 1);

        let sym = structs[0];
        assert_eq!(sym.name, "Point");
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert_eq!(sym.children.len(), 2);
        assert_eq!(sym.children[0].name, "x");
        assert_eq!(sym.children[0].kind, SymbolKind::Field);
        assert_eq!(sym.children[1].name, "y");
    }

    #[test]
    fn test_extract_typedef_enum() {
        let source = r#"
typedef enum {
    RED,
    GREEN,
    BLUE
} Color;
"#;
        let parsed = parse_source(Path::new("test.c"), source, Language::C).unwrap();
        let enums: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert_eq!(enums.len(), 1);

        let sym = enums[0];
        assert_eq!(sym.name, "Color");
        assert_eq!(sym.kind, SymbolKind::Enum);
        assert_eq!(sym.children.len(), 3);
        assert_eq!(sym.children[0].name, "RED");
        assert_eq!(sym.children[1].name, "GREEN");
        assert_eq!(sym.children[2].name, "BLUE");
    }

    #[test]
    fn test_extract_macro() {
        let source = r#"
#define MAX_SIZE 1024
#define SQUARE(x) ((x) * (x))
"#;
        let parsed = parse_source(Path::new("test.c"), source, Language::C).unwrap();
        let macros: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.name == "MAX_SIZE" || s.name == "SQUARE")
            .collect();
        assert_eq!(macros.len(), 2);
        assert_eq!(macros[0].name, "MAX_SIZE");
        assert_eq!(macros[1].name, "SQUARE");
    }

    #[test]
    fn test_extract_typedef_alias() {
        let source = r#"
typedef unsigned long size_type;
"#;
        let parsed = parse_source(Path::new("test.c"), source, Language::C).unwrap();
        let aliases: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].name, "size_type");
    }

    #[test]
    fn test_extract_includes() {
        let source = r#"
#include <stdio.h>
#include "myheader.h"

int main(void) {
    return 0;
}
"#;
        let parsed = parse_source(Path::new("test.c"), source, Language::C).unwrap();
        assert_eq!(parsed.imports.len(), 2);
        assert!(parsed.imports[0].contains("stdio.h"));
        assert!(parsed.imports[1].contains("myheader.h"));
    }
}
