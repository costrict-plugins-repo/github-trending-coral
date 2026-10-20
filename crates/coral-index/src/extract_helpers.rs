//! Shared helper functions for tree-sitter symbol extractors.
//!
//! These utilities are used across multiple language extractors to avoid
//! code duplication.

use crate::parser::get_node_text;

/// Get the text of a named field child node.
///
/// Equivalent to `node.child_by_field_name(field_name)` followed by
/// extracting the text as an owned `String`.
pub(crate) fn find_child_text(
    node: &tree_sitter::Node,
    field_name: &str,
    source: &str,
) -> Option<String> {
    node.child_by_field_name(field_name)
        .and_then(|n| get_node_text(&n, source))
        .map(|s| s.to_string())
}

/// Find the first child of `node` whose kind matches `kind`, and return its text.
pub(crate) fn find_child_by_kind_text(
    node: &tree_sitter::Node,
    kind: &str,
    source: &str,
) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return get_node_text(&child, source).map(|s| s.to_string());
        }
    }
    None
}
