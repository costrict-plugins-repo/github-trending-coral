use tree_sitter::Tree;

use crate::extract_helpers::find_child_text;
use crate::parser::get_node_text;
use crate::symbol::{Span, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a C++ source file's AST.
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
        "class_specifier" | "struct_specifier" => {
            // Standalone class/struct (not inside a declaration)
            if let Some(sym) = extract_class_or_struct(node, source) {
                symbols.push(sym);
            }
        }
        "namespace_definition" => {
            if let Some(sym) = extract_namespace(node, source) {
                symbols.push(sym);
            }
        }
        "template_declaration" => {
            extract_template_declaration(node, source, symbols);
        }
        "type_definition" => {
            if let Some(sym) = extract_typedef(node, source) {
                symbols.push(sym);
            }
        }
        "preproc_function_def" | "preproc_def" => {
            if let Some(sym) = extract_macro(node, source) {
                symbols.push(sym);
            }
        }
        "enum_specifier" => {
            if let Some(sym) = extract_enum(node, source) {
                symbols.push(sym);
            }
        }
        _ => {}
    }
}

fn extract_function(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = extract_cpp_function_name(node, source)?;
    let visibility = cpp_top_level_visibility(node, source);
    let doc_comment = extract_cpp_doc_comment(node, source);
    let signature = build_cpp_function_signature(node, source);

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

/// Extract the function/method name from a `function_definition` node.
fn extract_cpp_function_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    extract_declarator_name(&declarator, source)
}

/// Recursively extract the identifier name from a declarator chain.
fn extract_declarator_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => {
            get_node_text(node, source).map(|s| s.to_string())
        }
        "qualified_identifier" => {
            // e.g., `ClassName::method` — take the rightmost name
            node.child_by_field_name("name")
                .and_then(|n| get_node_text(&n, source))
                .map(|s| s.to_string())
        }
        "destructor_name" => {
            get_node_text(node, source).map(|s| s.to_string())
        }
        "operator_name" => {
            get_node_text(node, source).map(|s| s.to_string())
        }
        "function_declarator" | "pointer_declarator" | "reference_declarator"
        | "array_declarator" | "parenthesized_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_declarator_name(&inner, source)
        }
        _ => {
            // Try the "declarator" field
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
    // A top-level `declaration` may contain class_specifier, struct_specifier,
    // enum_specifier, or be a variable/function prototype.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_specifier" => {
                if let Some(sym) = extract_class_or_struct(&child, source) {
                    symbols.push(sym);
                }
            }
            "struct_specifier" => {
                if let Some(sym) = extract_class_or_struct(&child, source) {
                    symbols.push(sym);
                }
            }
            "enum_specifier" => {
                if let Some(sym) = extract_enum(&child, source) {
                    symbols.push(sym);
                }
            }
            _ => {}
        }
    }
}

fn extract_class_or_struct(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let doc_comment = extract_cpp_doc_comment(node, source);
    let is_class = node.kind() == "class_specifier";
    let kind = if is_class {
        SymbolKind::Class
    } else {
        SymbolKind::Struct
    };

    let children = extract_class_body(node, source, is_class);

    Some(Symbol {
        name,
        kind,
        span: Span::from_node(node),
        signature: None,
        visibility: Visibility::Public,
        doc_comment,
        children,
    })
}

fn extract_class_body(
    node: &tree_sitter::Node,
    source: &str,
    is_class: bool,
) -> Vec<Symbol> {
    let mut members = Vec::new();
    let mut cursor = node.walk();
    // C++ class defaults to private, struct defaults to public
    let mut current_visibility = if is_class {
        Visibility::Private
    } else {
        Visibility::Public
    };

    for child in node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut body_cursor = child.walk();
            for item in child.children(&mut body_cursor) {
                match item.kind() {
                    "access_specifier" => {
                        current_visibility = parse_access_specifier(&item, source);
                    }
                    "function_definition" => {
                        // Inline method definition
                        if let Some(mut sym) = extract_method_from_definition(&item, source) {
                            sym.visibility = current_visibility;
                            members.push(sym);
                        }
                    }
                    "declaration" => {
                        // Could be a method declaration or a field
                        extract_class_member_declaration(
                            &item,
                            source,
                            current_visibility,
                            &mut members,
                        );
                    }
                    "field_declaration" => {
                        extract_field_declaration(
                            &item,
                            source,
                            current_visibility,
                            &mut members,
                        );
                    }
                    "template_declaration" => {
                        // Templated method inside a class
                        let mut tmpl_cursor = item.walk();
                        for tmpl_child in item.children(&mut tmpl_cursor) {
                            if tmpl_child.kind() == "function_definition" {
                                if let Some(mut sym) =
                                    extract_method_from_definition(&tmpl_child, source)
                                {
                                    sym.visibility = current_visibility;
                                    members.push(sym);
                                }
                            } else if tmpl_child.kind() == "declaration" {
                                extract_class_member_declaration(
                                    &tmpl_child,
                                    source,
                                    current_visibility,
                                    &mut members,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    members
}

fn extract_method_from_definition(
    node: &tree_sitter::Node,
    source: &str,
) -> Option<Symbol> {
    let name = extract_cpp_function_name(node, source)?;
    let doc_comment = extract_cpp_doc_comment(node, source);
    let signature = build_cpp_function_signature(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Method,
        span: Span::from_node(node),
        signature: Some(signature),
        visibility: Visibility::Public, // will be overridden by caller
        doc_comment,
        children: Vec::new(),
    })
}

fn extract_class_member_declaration(
    node: &tree_sitter::Node,
    source: &str,
    visibility: Visibility,
    members: &mut Vec<Symbol>,
) {
    // Check if this declaration contains a function_declarator (method declaration)
    let mut cursor = node.walk();
    let mut has_function_declarator = false;

    for child in node.children(&mut cursor) {
        if child.kind() == "function_declarator" {
            has_function_declarator = true;
            if let Some(name) = extract_declarator_name(&child, source) {
                let signature = get_node_text(node, source)
                    .map(|t| t.trim().trim_end_matches(';').to_string());
                members.push(Symbol {
                    name,
                    kind: SymbolKind::Method,
                    span: Span::from_node(node),
                    signature,
                    visibility,
                    doc_comment: None,
                    children: Vec::new(),
                });
            }
        }
        if child.kind() == "init_declarator" {
            // Could contain a function_declarator inside
            let mut inner_cursor = child.walk();
            for inner_child in child.children(&mut inner_cursor) {
                if inner_child.kind() == "function_declarator" {
                    has_function_declarator = true;
                    if let Some(name) = extract_declarator_name(&inner_child, source) {
                        let signature = get_node_text(node, source)
                            .map(|t| t.trim().trim_end_matches(';').to_string());
                        members.push(Symbol {
                            name,
                            kind: SymbolKind::Method,
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
    }

    // If no function declarator found, this is likely a field (variable) declaration
    if !has_function_declarator {
        let mut cursor2 = node.walk();
        for child in node.children(&mut cursor2) {
            if child.kind() == "init_declarator" {
                if let Some(decl) = child.child_by_field_name("declarator") {
                    if let Some(name) = extract_declarator_name(&decl, source) {
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
    }
}

fn extract_field_declaration(
    node: &tree_sitter::Node,
    source: &str,
    visibility: Visibility,
    members: &mut Vec<Symbol>,
) {
    // field_declaration in C++ AST contains field_identifier children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "field_identifier" => {
                if let Some(name) = get_node_text(&child, source) {
                    let signature = get_node_text(node, source)
                        .map(|t| t.trim().trim_end_matches(';').to_string());
                    members.push(Symbol {
                        name: name.to_string(),
                        kind: SymbolKind::Field,
                        span: Span::from_node(node),
                        signature,
                        visibility,
                        doc_comment: None,
                        children: Vec::new(),
                    });
                }
            }
            "function_declarator" => {
                // Method declaration inside field_declaration
                if let Some(name) = extract_declarator_name(&child, source) {
                    let signature = get_node_text(node, source)
                        .map(|t| t.trim().trim_end_matches(';').to_string());
                    members.push(Symbol {
                        name,
                        kind: SymbolKind::Method,
                        span: Span::from_node(node),
                        signature,
                        visibility,
                        doc_comment: None,
                        children: Vec::new(),
                    });
                }
            }
            "pointer_declarator" | "array_declarator" => {
                if let Some(name) = extract_field_declarator_name(&child, source) {
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
            _ => {}
        }
    }
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

fn extract_namespace(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let doc_comment = extract_cpp_doc_comment(node, source);

    // Extract symbols from the namespace body
    let mut children = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            extract_top_level_node(&child, source, &mut children);
        }
    }

    Some(Symbol {
        name,
        kind: SymbolKind::Module, // namespaces map to Module
        span: Span::from_node(node),
        signature: None,
        visibility: Visibility::Public,
        doc_comment,
        children,
    })
}

fn extract_template_declaration(
    node: &tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<Symbol>,
) {
    // Template declarations wrap the actual declaration
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sym) = extract_function(&child, source) {
                    symbols.push(sym);
                }
            }
            "class_specifier" | "struct_specifier" => {
                if let Some(sym) = extract_class_or_struct(&child, source) {
                    symbols.push(sym);
                }
            }
            "declaration" => {
                extract_declaration(&child, source, symbols);
            }
            _ => {}
        }
    }
}

fn extract_typedef(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let doc_comment = extract_cpp_doc_comment(node, source);

    // Check for struct/enum/class inside the typedef
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_specifier" | "class_specifier" => {
                let name = extract_typedef_name(node, source)?;
                let is_class = child.kind() == "class_specifier";
                let children = extract_class_body(&child, source, is_class);
                return Some(Symbol {
                    name,
                    kind: if is_class {
                        SymbolKind::Class
                    } else {
                        SymbolKind::Struct
                    },
                    span: Span::from_node(node),
                    signature: None,
                    visibility: Visibility::Public,
                    doc_comment,
                    children,
                });
            }
            "enum_specifier" => {
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

    // Plain typedef alias
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

fn extract_typedef_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut last_type_id = None;
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            last_type_id = get_node_text(&child, source).map(|s| s.to_string());
        }
    }
    last_type_id
}

fn extract_enum(node: &tree_sitter::Node, source: &str) -> Option<Symbol> {
    let name = find_child_text(node, "name", source)?;
    let doc_comment = extract_cpp_doc_comment(node, source);
    let variants = extract_enum_variants(node, source);

    Some(Symbol {
        name,
        kind: SymbolKind::Enum,
        span: Span::from_node(node),
        signature: None,
        visibility: Visibility::Public,
        doc_comment,
        children: variants,
    })
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
                            kind: SymbolKind::Const,
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
    let doc_comment = extract_cpp_doc_comment(node, source);
    let signature = get_node_text(node, source).map(|t| {
        t.lines().next().unwrap_or("").trim().to_string()
    });

    Some(Symbol {
        name,
        kind: SymbolKind::Const,
        span: Span::from_node(node),
        signature,
        visibility: Visibility::Public,
        doc_comment,
        children: Vec::new(),
    })
}

// --- Helpers ---

fn parse_access_specifier(node: &tree_sitter::Node, source: &str) -> Visibility {
    let text = get_node_text(node, source).unwrap_or("");
    if text.contains("public") {
        Visibility::Public
    } else if text.contains("protected") {
        Visibility::Crate // using Crate for protected visibility
    } else {
        Visibility::Private
    }
}

/// At file scope, `static` means internal linkage.
fn cpp_top_level_visibility(node: &tree_sitter::Node, source: &str) -> Visibility {
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

fn extract_cpp_doc_comment(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            if let Some(text) = get_node_text(&sibling, source) {
                let doc = if text.starts_with("/*") {
                    text.trim()
                        .trim_start_matches("/**")
                        .trim_start_matches("/*")
                        .trim_end_matches("*/")
                        .lines()
                        .map(|line| line.trim().trim_start_matches('*').trim())
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    // /// or // style
                    text.trim_start_matches("///")
                        .trim_start_matches("//")
                        .trim()
                        .to_string()
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

fn build_cpp_function_signature(node: &tree_sitter::Node, source: &str) -> String {
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
    use crate::symbol::{SymbolKind, Visibility};
    use std::path::Path;

    #[test]
    fn test_extract_function() {
        let source = r#"
/// Compute the sum of two integers.
int add(int a, int b) {
    return a + b;
}
"#;
        let parsed = parse_source(Path::new("test.cpp"), source, Language::Cpp).unwrap();
        let funcs: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);

        let sym = funcs[0];
        assert_eq!(sym.name, "add");
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.signature.as_ref().unwrap().contains("int add"));
        assert!(sym
            .doc_comment
            .as_ref()
            .unwrap()
            .contains("Compute the sum"));
    }

    #[test]
    fn test_extract_class() {
        let source = r#"
class Shape {
public:
    int width;
    int height;

    void draw();

    int area() {
        return width * height;
    }

private:
    int id;
};
"#;
        let parsed = parse_source(Path::new("test.cpp"), source, Language::Cpp).unwrap();
        let classes: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);

        let sym = classes[0];
        assert_eq!(sym.name, "Shape");
        assert_eq!(sym.kind, SymbolKind::Class);

        let fields: Vec<_> = sym
            .children
            .iter()
            .filter(|c| c.kind == SymbolKind::Field)
            .collect();
        let methods: Vec<_> = sym
            .children
            .iter()
            .filter(|c| c.kind == SymbolKind::Method)
            .collect();

        // public fields: width, height; private field: id
        assert!(fields.iter().any(|f| f.name == "width" && f.visibility == Visibility::Public));
        assert!(fields.iter().any(|f| f.name == "height" && f.visibility == Visibility::Public));
        assert!(fields.iter().any(|f| f.name == "id" && f.visibility == Visibility::Private));

        // Methods: draw (declaration), area (definition)
        assert!(methods.iter().any(|m| m.name == "draw"));
        assert!(methods.iter().any(|m| m.name == "area"));
    }

    #[test]
    fn test_extract_namespace() {
        let source = r#"
namespace math {
    int add(int a, int b) {
        return a + b;
    }

    int subtract(int a, int b) {
        return a - b;
    }
}
"#;
        let parsed = parse_source(Path::new("test.cpp"), source, Language::Cpp).unwrap();
        let namespaces: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Module)
            .collect();
        assert_eq!(namespaces.len(), 1);

        let ns = namespaces[0];
        assert_eq!(ns.name, "math");
        assert_eq!(ns.children.len(), 2);
        assert_eq!(ns.children[0].name, "add");
        assert_eq!(ns.children[0].kind, SymbolKind::Function);
        assert_eq!(ns.children[1].name, "subtract");
    }

    #[test]
    fn test_extract_enum_class() {
        let source = r#"
enum class Color {
    Red,
    Green,
    Blue
};
"#;
        let parsed = parse_source(Path::new("test.cpp"), source, Language::Cpp).unwrap();
        let enums: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert_eq!(enums.len(), 1);

        let sym = enums[0];
        assert_eq!(sym.name, "Color");
        assert_eq!(sym.children.len(), 3);
        assert_eq!(sym.children[0].name, "Red");
        assert_eq!(sym.children[1].name, "Green");
        assert_eq!(sym.children[2].name, "Blue");
    }

    #[test]
    fn test_extract_struct_with_default_visibility() {
        let source = r#"
struct Point {
    double x;
    double y;
};
"#;
        let parsed = parse_source(Path::new("test.cpp"), source, Language::Cpp).unwrap();
        let structs: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert_eq!(structs.len(), 1);

        let sym = structs[0];
        assert_eq!(sym.name, "Point");

        // Struct members should default to public
        let fields: Vec<_> = sym
            .children
            .iter()
            .filter(|c| c.kind == SymbolKind::Field)
            .collect();
        assert!(fields.len() >= 2);
        for field in &fields {
            assert_eq!(field.visibility, Visibility::Public);
        }
    }

    #[test]
    fn test_extract_template_function() {
        let source = r#"
template<typename T>
T maximum(T a, T b) {
    return (a > b) ? a : b;
}
"#;
        let parsed = parse_source(Path::new("test.cpp"), source, Language::Cpp).unwrap();
        let funcs: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "maximum");
    }

    #[test]
    fn test_extract_includes() {
        let source = r#"
#include <iostream>
#include <vector>
#include "myheader.hpp"

int main() {
    return 0;
}
"#;
        let parsed = parse_source(Path::new("test.cpp"), source, Language::Cpp).unwrap();
        assert_eq!(parsed.imports.len(), 3);
        assert!(parsed.imports[0].contains("iostream"));
        assert!(parsed.imports[1].contains("vector"));
        assert!(parsed.imports[2].contains("myheader.hpp"));
    }
}
