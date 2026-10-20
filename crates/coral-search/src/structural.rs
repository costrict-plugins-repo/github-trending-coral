//! Structural search — graph-based code retrieval.
//!
//! Given a query (symbol name), finds related symbols via the dependency
//! graph: direct matches, callers, callees, and symbols in affected files.

use std::collections::HashSet;
use std::path::PathBuf;

use coral_graph::CodeGraph;

use crate::types::{SearchResult, SearchSource};

/// Run a structural search against the code graph.
///
/// Strategy:
/// 1. Find direct symbol matches in the graph
/// 2. Get callers and callees of matched symbols
/// 3. Score: direct match = 1.0, caller/callee = 0.7
pub fn structural_search(graph: &CodeGraph, query: &str, limit: usize) -> Vec<SearchResult> {
    let mut results: Vec<SearchResult> = Vec::new();
    let mut seen: HashSet<(PathBuf, String)> = HashSet::new();

    let matches = graph.find_symbol(query);

    // Direct matches — score 1.0
    for node in &matches {
        let key = (node.file.clone(), node.id.symbol.clone());
        if seen.insert(key) {
            results.push(SearchResult {
                path: node.file.clone(),
                symbol: Some(node.id.symbol.clone()),
                snippet: node
                    .signature
                    .clone()
                    .unwrap_or_else(|| node.id.symbol.clone()),
                score: 1.0,
                source: SearchSource::Structural,
                line: Some(node.line),
            });
        }
    }

    // For each direct match, add callers and callees — score 0.7
    for node in &matches {
        for caller in graph.callers(&node.id) {
            let key = (caller.file.clone(), caller.id.symbol.clone());
            if seen.insert(key) {
                results.push(SearchResult {
                    path: caller.file.clone(),
                    symbol: Some(caller.id.symbol.clone()),
                    snippet: caller
                        .signature
                        .clone()
                        .unwrap_or_else(|| caller.id.symbol.clone()),
                    score: 0.7,
                    source: SearchSource::Structural,
                    line: Some(caller.line),
                });
            }
        }
        for callee in graph.callees(&node.id) {
            let key = (callee.file.clone(), callee.id.symbol.clone());
            if seen.insert(key) {
                results.push(SearchResult {
                    path: callee.file.clone(),
                    symbol: Some(callee.id.symbol.clone()),
                    snippet: callee
                        .signature
                        .clone()
                        .unwrap_or_else(|| callee.id.symbol.clone()),
                    score: 0.7,
                    source: SearchSource::Structural,
                    line: Some(callee.line),
                });
            }
        }
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_graph::builder::{build_graph, FileSymbols, ReferenceInfo, SymbolInfo};
    use coral_graph::types::{EdgeKind, NodeKind};

    fn sample_graph() -> CodeGraph {
        let files = vec![
            FileSymbols {
                path: PathBuf::from("src/auth.rs"),
                symbols: vec![
                    SymbolInfo {
                        name: "authenticate".to_string(),
                        kind: NodeKind::Function,
                        signature: Some("pub fn authenticate(token: &str) -> Result<User>".to_string()),
                        line: 5,
                        children: vec![],
                    },
                    SymbolInfo {
                        name: "validate_jwt".to_string(),
                        kind: NodeKind::Function,
                        signature: Some("fn validate_jwt(token: &str) -> bool".to_string()),
                        line: 20,
                        children: vec![],
                    },
                ],
                imports: vec![],
                references: vec![ReferenceInfo {
                    from_symbol: "authenticate".to_string(),
                    to_symbol: "validate_jwt".to_string(),
                    kind: EdgeKind::Calls,
                }],
            },
            FileSymbols {
                path: PathBuf::from("src/server.rs"),
                symbols: vec![SymbolInfo {
                    name: "handle_request".to_string(),
                    kind: NodeKind::Function,
                    signature: Some("pub fn handle_request(req: Request) -> Response".to_string()),
                    line: 1,
                    children: vec![],
                }],
                imports: vec![],
                references: vec![ReferenceInfo {
                    from_symbol: "handle_request".to_string(),
                    to_symbol: "authenticate".to_string(),
                    kind: EdgeKind::Calls,
                }],
            },
        ];
        build_graph(&files)
    }

    #[test]
    fn test_structural_direct_match() {
        let graph = sample_graph();
        let results = structural_search(&graph, "authenticate", 10);

        assert!(!results.is_empty(), "should find the authenticate symbol");
        assert_eq!(results[0].symbol.as_deref(), Some("authenticate"));
        assert_eq!(results[0].score, 1.0);
        assert_eq!(results[0].source, SearchSource::Structural);
    }

    #[test]
    fn test_structural_includes_callers_and_callees() {
        let graph = sample_graph();
        let results = structural_search(&graph, "authenticate", 10);

        let symbols: Vec<&str> = results
            .iter()
            .filter_map(|r| r.symbol.as_deref())
            .collect();

        // Direct match
        assert!(symbols.contains(&"authenticate"));
        // Caller
        assert!(
            symbols.contains(&"handle_request"),
            "should include caller handle_request"
        );
        // Callee
        assert!(
            symbols.contains(&"validate_jwt"),
            "should include callee validate_jwt"
        );
    }

    #[test]
    fn test_structural_scores_ordering() {
        let graph = sample_graph();
        let results = structural_search(&graph, "authenticate", 10);

        // Direct match (1.0) should come before callers/callees (0.7)
        assert!(results.len() >= 2);
        assert!(results[0].score >= results[1].score);
    }

    #[test]
    fn test_structural_no_match() {
        let graph = sample_graph();
        let results = structural_search(&graph, "nonexistent_symbol", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_structural_respects_limit() {
        let graph = sample_graph();
        let results = structural_search(&graph, "authenticate", 1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_structural_no_duplicates() {
        let graph = sample_graph();
        let results = structural_search(&graph, "authenticate", 10);

        let mut seen = HashSet::new();
        for r in &results {
            let key = (r.path.clone(), r.symbol.clone());
            assert!(seen.insert(key), "duplicate result found");
        }
    }
}
