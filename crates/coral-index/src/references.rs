//! Call reference extraction from function bodies.
//!
//! Walks function/method bodies looking for call expressions and records
//! `CallReference` entries that pair the enclosing function with the callee.

use tree_sitter::Tree;

use crate::parser::{Language, get_node_text};
use crate::symbol::CallReference;

/// Extract call references from all functions in the tree.
pub fn extract_references(tree: &Tree, source: &str, language: Language) -> Vec<CallReference> {
    match language {
        Language::Rust => extract_rust_references(tree, source),
        Language::TypeScript | Language::JavaScript => extract_ts_references(tree, source),
        Language::Python => extract_python_references(tree, source),
        #[cfg(feature = "lang-go")]
        Language::Go => extract_go_references(tree, source),
        #[cfg(feature = "lang-java")]
        Language::Java => extract_java_references(tree, source),
        #[cfg(feature = "lang-c")]
        Language::C => extract_c_references(tree, source),
        #[cfg(feature = "lang-cpp")]
        Language::Cpp => extract_cpp_references(tree, source),
    }
}

// ─── Rust ────────────────────────────────────────────────────────────────────

fn extract_rust_references(tree: &Tree, source: &str) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name) = child.child_by_field_name("name") {
                    if let Some(fn_name) = get_node_text(&name, source) {
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_rust_calls(&body, fn_name, source, &mut refs);
                        }
                    }
                }
            }
            "impl_item" => {
                // Walk methods inside impl blocks
                let mut impl_cursor = child.walk();
                for impl_child in child.children(&mut impl_cursor) {
                    if impl_child.kind() == "declaration_list" {
                        let mut decl_cursor = impl_child.walk();
                        for item in impl_child.children(&mut decl_cursor) {
                            if item.kind() == "function_item" {
                                if let Some(name) = item.child_by_field_name("name") {
                                    if let Some(fn_name) = get_node_text(&name, source) {
                                        if let Some(body) = item.child_by_field_name("body") {
                                            collect_rust_calls(&body, fn_name, source, &mut refs);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    refs
}

/// Recursively walk a Rust node looking for `call_expression` nodes.
fn collect_rust_calls(
    node: &tree_sitter::Node,
    from_symbol: &str,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(callee) = extract_rust_callee(&child, source) {
                refs.push(CallReference {
                    from_symbol: from_symbol.to_string(),
                    to_symbol: callee,
                    line: child.start_position().row as u32,
                });
            }
        }
        // Recurse into children
        collect_rust_calls(&child, from_symbol, source, refs);
    }
}

/// Extract the callee name from a Rust call_expression.
///
/// Handles:
/// - Simple calls: `foo()`           -> "foo"
/// - Method calls: `self.foo()`      -> "foo"
/// - Path calls: `module::func()`    -> "func"
/// - Qualified: `Vec::new()`         -> "new"
fn extract_rust_callee(call_node: &tree_sitter::Node, source: &str) -> Option<String> {
    let callee = call_node.child_by_field_name("function")?;

    match callee.kind() {
        "identifier" => get_node_text(&callee, source).map(|s| s.to_string()),
        "field_expression" => {
            // e.g., `self.foo()` or `obj.method()`
            callee
                .child_by_field_name("field")
                .and_then(|f| get_node_text(&f, source))
                .map(|s| s.to_string())
        }
        "scoped_identifier" => {
            // e.g., `module::func()` or `Type::method()`
            callee
                .child_by_field_name("name")
                .and_then(|n| get_node_text(&n, source))
                .map(|s| s.to_string())
        }
        "generic_function" => {
            // e.g., `foo::<T>()`
            let mut cursor = callee.walk();
            for child in callee.children(&mut cursor) {
                if child.kind() == "identifier" || child.kind() == "scoped_identifier" {
                    if child.kind() == "scoped_identifier" {
                        return child
                            .child_by_field_name("name")
                            .and_then(|n| get_node_text(&n, source))
                            .map(|s| s.to_string());
                    }
                    return get_node_text(&child, source).map(|s| s.to_string());
                }
            }
            None
        }
        _ => {
            // Fallback: try to get some text
            get_node_text(&callee, source).map(|s| {
                // Take the last segment for complex expressions
                s.rsplit("::").next().unwrap_or(s).to_string()
            })
        }
    }
}

// ─── TypeScript / JavaScript ─────────────────────────────────────────────────

fn extract_ts_references(tree: &Tree, source: &str) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = find_child_by_kind(&child, "identifier") {
                    if let Some(fn_name) = get_node_text(&name_node, source) {
                        if let Some(body) = find_child_by_kind(&child, "statement_block") {
                            collect_ts_calls(&body, fn_name, source, &mut refs);
                        }
                    }
                }
            }
            "lexical_declaration" => {
                // const foo = () => { ... } or const foo = function() { ... }
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "variable_declarator" {
                        if let Some(name_node) = find_child_by_kind(&decl_child, "identifier") {
                            if let Some(fn_name) = get_node_text(&name_node, source) {
                                if let Some(value) = decl_child.child_by_field_name("value") {
                                    if value.kind() == "arrow_function"
                                        || value.kind() == "function"
                                    {
                                        if let Some(body) =
                                            find_child_by_kind(&value, "statement_block")
                                        {
                                            collect_ts_calls(&body, fn_name, source, &mut refs);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "class_declaration" => {
                // Walk methods inside class body
                if let Some(body) = find_child_by_kind(&child, "class_body") {
                    let mut body_cursor = body.walk();
                    for member in body.children(&mut body_cursor) {
                        if member.kind() == "method_definition" {
                            if let Some(name_node) =
                                find_child_by_kind(&member, "property_identifier")
                            {
                                if let Some(method_name) = get_node_text(&name_node, source) {
                                    if let Some(method_body) =
                                        find_child_by_kind(&member, "statement_block")
                                    {
                                        collect_ts_calls(
                                            &method_body,
                                            method_name,
                                            source,
                                            &mut refs,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "export_statement" => {
                // Unwrap exported declarations
                let mut export_cursor = child.walk();
                for export_child in child.children(&mut export_cursor) {
                    match export_child.kind() {
                        "function_declaration" => {
                            if let Some(name_node) = find_child_by_kind(&export_child, "identifier")
                            {
                                if let Some(fn_name) = get_node_text(&name_node, source) {
                                    if let Some(body) =
                                        find_child_by_kind(&export_child, "statement_block")
                                    {
                                        collect_ts_calls(&body, fn_name, source, &mut refs);
                                    }
                                }
                            }
                        }
                        "class_declaration" => {
                            if let Some(body) = find_child_by_kind(&export_child, "class_body") {
                                let mut body_cursor = body.walk();
                                for member in body.children(&mut body_cursor) {
                                    if member.kind() == "method_definition" {
                                        if let Some(name_node) =
                                            find_child_by_kind(&member, "property_identifier")
                                        {
                                            if let Some(method_name) =
                                                get_node_text(&name_node, source)
                                            {
                                                if let Some(method_body) =
                                                    find_child_by_kind(&member, "statement_block")
                                                {
                                                    collect_ts_calls(
                                                        &method_body,
                                                        method_name,
                                                        source,
                                                        &mut refs,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    refs
}

/// Recursively walk a TS/JS node looking for `call_expression` nodes.
fn collect_ts_calls(
    node: &tree_sitter::Node,
    from_symbol: &str,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(callee) = extract_ts_callee(&child, source) {
                refs.push(CallReference {
                    from_symbol: from_symbol.to_string(),
                    to_symbol: callee,
                    line: child.start_position().row as u32,
                });
            }
        }
        // Recurse
        collect_ts_calls(&child, from_symbol, source, refs);
    }
}

/// Extract the callee name from a TS/JS call_expression.
///
/// Handles:
/// - Simple calls: `foo()`           -> "foo"
/// - Method calls: `obj.method()`    -> "method"
/// - Chained: `this.service.get()`   -> "get"
fn extract_ts_callee(call_node: &tree_sitter::Node, source: &str) -> Option<String> {
    let callee = call_node.child_by_field_name("function")?;

    match callee.kind() {
        "identifier" => get_node_text(&callee, source).map(|s| s.to_string()),
        "member_expression" => {
            // e.g., `obj.method()` or `this.foo()`
            callee
                .child_by_field_name("property")
                .and_then(|p| get_node_text(&p, source))
                .map(|s| s.to_string())
        }
        _ => {
            // Fallback: try to get the last identifier-like text
            get_node_text(&callee, source).map(|s| {
                s.rsplit('.').next().unwrap_or(s).to_string()
            })
        }
    }
}

// ─── Python ──────────────────────────────────────────────────────────────────

fn extract_python_references(tree: &Tree, source: &str) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name) = child.child_by_field_name("name") {
                    if let Some(fn_name) = get_node_text(&name, source) {
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_python_calls(&body, fn_name, source, &mut refs);
                        }
                    }
                }
            }
            "class_definition" => {
                // Walk methods inside class body
                if let Some(body) = child.child_by_field_name("body") {
                    let mut body_cursor = body.walk();
                    for item in body.children(&mut body_cursor) {
                        if item.kind() == "function_definition" {
                            if let Some(name) = item.child_by_field_name("name") {
                                if let Some(method_name) = get_node_text(&name, source) {
                                    if let Some(method_body) = item.child_by_field_name("body") {
                                        collect_python_calls(
                                            &method_body,
                                            method_name,
                                            source,
                                            &mut refs,
                                        );
                                    }
                                }
                            }
                        } else if item.kind() == "decorated_definition" {
                            // Decorated methods
                            let mut deco_cursor = item.walk();
                            for deco_child in item.children(&mut deco_cursor) {
                                if deco_child.kind() == "function_definition" {
                                    if let Some(name) = deco_child.child_by_field_name("name") {
                                        if let Some(method_name) = get_node_text(&name, source) {
                                            if let Some(method_body) =
                                                deco_child.child_by_field_name("body")
                                            {
                                                collect_python_calls(
                                                    &method_body,
                                                    method_name,
                                                    source,
                                                    &mut refs,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "decorated_definition" => {
                // Decorated top-level functions
                let mut deco_cursor = child.walk();
                for deco_child in child.children(&mut deco_cursor) {
                    if deco_child.kind() == "function_definition" {
                        if let Some(name) = deco_child.child_by_field_name("name") {
                            if let Some(fn_name) = get_node_text(&name, source) {
                                if let Some(body) = deco_child.child_by_field_name("body") {
                                    collect_python_calls(&body, fn_name, source, &mut refs);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    refs
}

/// Recursively walk a Python node looking for `call` nodes.
fn collect_python_calls(
    node: &tree_sitter::Node,
    from_symbol: &str,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(callee) = extract_python_callee(&child, source) {
                refs.push(CallReference {
                    from_symbol: from_symbol.to_string(),
                    to_symbol: callee,
                    line: child.start_position().row as u32,
                });
            }
        }
        // Recurse
        collect_python_calls(&child, from_symbol, source, refs);
    }
}

/// Extract the callee name from a Python `call` node.
///
/// Handles:
/// - Simple calls: `foo()`           -> "foo"
/// - Method calls: `self.foo()`      -> "foo"
/// - Attribute calls: `obj.method()` -> "method"
fn extract_python_callee(call_node: &tree_sitter::Node, source: &str) -> Option<String> {
    let callee = call_node.child_by_field_name("function")?;

    match callee.kind() {
        "identifier" => get_node_text(&callee, source).map(|s| s.to_string()),
        "attribute" => {
            // e.g., `self.foo()` or `obj.method()`
            callee
                .child_by_field_name("attribute")
                .and_then(|a| get_node_text(&a, source))
                .map(|s| s.to_string())
        }
        _ => {
            // Fallback
            get_node_text(&callee, source).map(|s| {
                s.rsplit('.').next().unwrap_or(s).to_string()
            })
        }
    }
}

// ─── Go ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "lang-go")]
fn extract_go_references(tree: &Tree, source: &str) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = child.child_by_field_name("name") {
                    if let Some(fn_name) = get_node_text(&name, source) {
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_go_calls(&body, fn_name, source, &mut refs);
                        }
                    }
                }
            }
            "method_declaration" => {
                if let Some(name) = child.child_by_field_name("name") {
                    if let Some(fn_name) = get_node_text(&name, source) {
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_go_calls(&body, fn_name, source, &mut refs);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    refs
}

#[cfg(feature = "lang-go")]
/// Recursively walk a Go node looking for `call_expression` nodes.
fn collect_go_calls(
    node: &tree_sitter::Node,
    from_symbol: &str,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(callee) = extract_go_callee(&child, source) {
                refs.push(CallReference {
                    from_symbol: from_symbol.to_string(),
                    to_symbol: callee,
                    line: child.start_position().row as u32,
                });
            }
        }
        collect_go_calls(&child, from_symbol, source, refs);
    }
}

#[cfg(feature = "lang-go")]
/// Extract the callee name from a Go call_expression.
fn extract_go_callee(call_node: &tree_sitter::Node, source: &str) -> Option<String> {
    let callee = call_node.child_by_field_name("function")?;

    match callee.kind() {
        "identifier" => get_node_text(&callee, source).map(|s| s.to_string()),
        "selector_expression" => {
            // e.g., `obj.Method()` or `pkg.Func()`
            callee
                .child_by_field_name("field")
                .and_then(|f| get_node_text(&f, source))
                .map(|s| s.to_string())
        }
        _ => {
            get_node_text(&callee, source).map(|s| {
                s.rsplit('.').next().unwrap_or(s).to_string()
            })
        }
    }
}

// ─── Java ───────────────────────────────────────────────────────────────────

#[cfg(feature = "lang-java")]
fn extract_java_references(tree: &Tree, source: &str) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    // Java top-level is typically class declarations
    for child in root.children(&mut cursor) {
        if child.kind() == "class_declaration" || child.kind() == "interface_declaration" {
            collect_java_class_refs(&child, source, &mut refs);
        }
    }

    refs
}

#[cfg(feature = "lang-java")]
fn collect_java_class_refs(
    node: &tree_sitter::Node,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_body" || child.kind() == "interface_body" {
            let mut body_cursor = child.walk();
            for member in child.children(&mut body_cursor) {
                if member.kind() == "method_declaration" || member.kind() == "constructor_declaration" {
                    if let Some(name) = member.child_by_field_name("name") {
                        if let Some(fn_name) = get_node_text(&name, source) {
                            if let Some(body) = member.child_by_field_name("body") {
                                collect_java_calls(&body, fn_name, source, refs);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(feature = "lang-java")]
/// Recursively walk a Java node looking for `method_invocation` nodes.
fn collect_java_calls(
    node: &tree_sitter::Node,
    from_symbol: &str,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "method_invocation" {
            if let Some(name) = child.child_by_field_name("name") {
                if let Some(callee) = get_node_text(&name, source) {
                    refs.push(CallReference {
                        from_symbol: from_symbol.to_string(),
                        to_symbol: callee.to_string(),
                        line: child.start_position().row as u32,
                    });
                }
            }
        }
        collect_java_calls(&child, from_symbol, source, refs);
    }
}

// ─── C ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "lang-c")]
fn extract_c_references(tree: &Tree, source: &str) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "function_definition" {
            if let Some(declarator) = child.child_by_field_name("declarator") {
                if let Some(name_node) = declarator.child_by_field_name("declarator") {
                    if let Some(fn_name) = get_node_text(&name_node, source) {
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_c_calls(&body, fn_name, source, &mut refs);
                        }
                    }
                }
            }
        }
    }

    refs
}

#[cfg(feature = "lang-c")]
/// Recursively walk a C node looking for `call_expression` nodes.
fn collect_c_calls(
    node: &tree_sitter::Node,
    from_symbol: &str,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(callee) = extract_c_callee(&child, source) {
                refs.push(CallReference {
                    from_symbol: from_symbol.to_string(),
                    to_symbol: callee,
                    line: child.start_position().row as u32,
                });
            }
        }
        collect_c_calls(&child, from_symbol, source, refs);
    }
}

#[cfg(feature = "lang-c")]
/// Extract the callee name from a C call_expression.
fn extract_c_callee(call_node: &tree_sitter::Node, source: &str) -> Option<String> {
    let callee = call_node.child_by_field_name("function")?;

    match callee.kind() {
        "identifier" => get_node_text(&callee, source).map(|s| s.to_string()),
        "field_expression" => {
            // e.g., `obj->method()` or `obj.func()`
            callee
                .child_by_field_name("field")
                .and_then(|f| get_node_text(&f, source))
                .map(|s| s.to_string())
        }
        _ => {
            get_node_text(&callee, source).map(|s| s.to_string())
        }
    }
}

// ─── C++ ───────────────────────────────────────────────────────────────────

#[cfg(feature = "lang-cpp")]
fn extract_cpp_references(tree: &Tree, source: &str) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    collect_cpp_top_level_refs(&root, source, &mut refs);
    refs
}

#[cfg(feature = "lang-cpp")]
fn collect_cpp_top_level_refs(
    node: &tree_sitter::Node,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(fn_name) = extract_cpp_function_name(&child, source) {
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_cpp_calls(&body, &fn_name, source, refs);
                    }
                }
            }
            "namespace_definition" => {
                // Recurse into namespace bodies
                if let Some(body) = child.child_by_field_name("body") {
                    collect_cpp_top_level_refs(&body, source, refs);
                }
            }
            "class_specifier" | "struct_specifier" => {
                // Walk methods inside class bodies
                if let Some(body) = child.child_by_field_name("body") {
                    collect_cpp_top_level_refs(&body, source, refs);
                }
            }
            "template_declaration" => {
                // Recurse into the templated declaration
                collect_cpp_top_level_refs(&child, source, refs);
            }
            _ => {}
        }
    }
}

#[cfg(feature = "lang-cpp")]
fn extract_cpp_function_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    // The declarator might be a function_declarator wrapping a qualified_identifier or identifier
    let inner = declarator.child_by_field_name("declarator")
        .unwrap_or(declarator);

    match inner.kind() {
        "identifier" => get_node_text(&inner, source).map(|s| s.to_string()),
        "qualified_identifier" => {
            // e.g., `ClassName::method` — take the rightmost name
            inner
                .child_by_field_name("name")
                .and_then(|n| get_node_text(&n, source))
                .map(|s| s.to_string())
        }
        "field_identifier" => get_node_text(&inner, source).map(|s| s.to_string()),
        "destructor_name" => get_node_text(&inner, source).map(|s| s.to_string()),
        _ => get_node_text(&inner, source).map(|s| s.to_string()),
    }
}

#[cfg(feature = "lang-cpp")]
/// Recursively walk a C++ node looking for `call_expression` nodes.
fn collect_cpp_calls(
    node: &tree_sitter::Node,
    from_symbol: &str,
    source: &str,
    refs: &mut Vec<CallReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(callee) = extract_cpp_callee(&child, source) {
                refs.push(CallReference {
                    from_symbol: from_symbol.to_string(),
                    to_symbol: callee,
                    line: child.start_position().row as u32,
                });
            }
        }
        collect_cpp_calls(&child, from_symbol, source, refs);
    }
}

#[cfg(feature = "lang-cpp")]
/// Extract the callee name from a C++ call_expression.
fn extract_cpp_callee(call_node: &tree_sitter::Node, source: &str) -> Option<String> {
    let callee = call_node.child_by_field_name("function")?;

    match callee.kind() {
        "identifier" => get_node_text(&callee, source).map(|s| s.to_string()),
        "qualified_identifier" => {
            // e.g., `std::sort()` — take the rightmost name
            callee
                .child_by_field_name("name")
                .and_then(|n| get_node_text(&n, source))
                .map(|s| s.to_string())
        }
        "field_expression" => {
            // e.g., `obj.method()` or `ptr->method()`
            callee
                .child_by_field_name("field")
                .and_then(|f| get_node_text(&f, source))
                .map(|s| s.to_string())
        }
        "template_function" => {
            // e.g., `make_shared<T>()`
            let mut inner_cursor = callee.walk();
            for inner_child in callee.children(&mut inner_cursor) {
                if inner_child.kind() == "identifier" || inner_child.kind() == "qualified_identifier" {
                    if inner_child.kind() == "qualified_identifier" {
                        return inner_child
                            .child_by_field_name("name")
                            .and_then(|n| get_node_text(&n, source))
                            .map(|s| s.to_string());
                    }
                    return get_node_text(&inner_child, source).map(|s| s.to_string());
                }
            }
            None
        }
        _ => {
            get_node_text(&callee, source).map(|s| {
                s.rsplit("::").next().unwrap_or(s).to_string()
            })
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn find_child_by_kind<'a>(
    node: &'a tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;
    use std::path::Path;

    #[test]
    fn test_rust_simple_call() {
        let source = r#"
fn foo() {
    bar();
}

fn bar() {}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        assert!(!parsed.references.is_empty());
        let r = &parsed.references[0];
        assert_eq!(r.from_symbol, "foo");
        assert_eq!(r.to_symbol, "bar");
    }

    #[test]
    fn test_rust_method_call() {
        let source = r#"
fn process() {
    let s = String::new();
    s.push_str("hello");
}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        // Should find both `new` and `push_str`
        let callees: Vec<&str> = parsed.references.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"new"));
        assert!(callees.contains(&"push_str"));
        // All from "process"
        assert!(parsed.references.iter().all(|r| r.from_symbol == "process"));
    }

    #[test]
    fn test_rust_impl_method_calls() {
        let source = r#"
impl Server {
    fn handle(&self) {
        self.validate();
        process_request();
    }

    fn validate(&self) {}
}

fn process_request() {}
"#;
        let parsed = parse_source(Path::new("test.rs"), source, Language::Rust).unwrap();
        let handle_refs: Vec<&CallReference> = parsed
            .references
            .iter()
            .filter(|r| r.from_symbol == "handle")
            .collect();
        let callees: Vec<&str> = handle_refs.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"validate"));
        assert!(callees.contains(&"process_request"));
    }

    #[test]
    fn test_ts_simple_call() {
        let source = r#"
function handleRequest() {
    validateToken();
    processData();
}

function validateToken() {}
function processData() {}
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        let refs: Vec<&CallReference> = parsed
            .references
            .iter()
            .filter(|r| r.from_symbol == "handleRequest")
            .collect();
        let callees: Vec<&str> = refs.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"validateToken"));
        assert!(callees.contains(&"processData"));
    }

    #[test]
    fn test_ts_method_call() {
        let source = r#"
function init() {
    const db = createConnection();
    db.connect();
}
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        let callees: Vec<&str> = parsed.references.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"createConnection"));
        assert!(callees.contains(&"connect"));
    }

    #[test]
    fn test_ts_arrow_function_calls() {
        let source = r#"
const process = () => {
    validate();
    transform();
};
"#;
        let parsed = parse_source(Path::new("test.ts"), source, Language::TypeScript).unwrap();
        let refs: Vec<&CallReference> = parsed
            .references
            .iter()
            .filter(|r| r.from_symbol == "process")
            .collect();
        let callees: Vec<&str> = refs.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"validate"));
        assert!(callees.contains(&"transform"));
    }

    #[test]
    fn test_python_simple_call() {
        let source = r#"
def process():
    check_input()
    transform_data()
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        let refs: Vec<&CallReference> = parsed
            .references
            .iter()
            .filter(|r| r.from_symbol == "process")
            .collect();
        let callees: Vec<&str> = refs.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"check_input"));
        assert!(callees.contains(&"transform_data"));
    }

    #[test]
    fn test_python_method_call() {
        let source = r#"
def handle():
    obj = Service()
    obj.run()
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        let callees: Vec<&str> = parsed.references.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"Service"));
        assert!(callees.contains(&"run"));
    }

    #[test]
    fn test_python_class_method_calls() {
        let source = r#"
class Handler:
    def process(self):
        self.validate()
        transform()

    def validate(self):
        pass

def transform():
    pass
"#;
        let parsed = parse_source(Path::new("test.py"), source, Language::Python).unwrap();
        let process_refs: Vec<&CallReference> = parsed
            .references
            .iter()
            .filter(|r| r.from_symbol == "process")
            .collect();
        let callees: Vec<&str> = process_refs.iter().map(|r| r.to_symbol.as_str()).collect();
        assert!(callees.contains(&"validate"));
        assert!(callees.contains(&"transform"));
    }
}
