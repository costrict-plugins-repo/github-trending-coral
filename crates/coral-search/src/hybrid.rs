use std::collections::HashMap;
use std::path::PathBuf;

use crate::types::SearchResult;

/// Fuse results from multiple search sources using Reciprocal Rank Fusion (RRF).
///
/// Each input is a ranked list of results from one search method.
/// RRF score for a document = sum(1 / (k + rank_in_set)) for each set that contains it.
/// Documents are identified by (path, symbol, line) tuple.
///
/// # Arguments
/// * `result_sets` - Ranked result lists from different search methods.
/// * `k` - Smoothing constant (typically 60).
/// * `limit` - Maximum number of results to return.
pub fn reciprocal_rank_fusion(
    result_sets: Vec<Vec<SearchResult>>,
    k: f64,
    limit: usize,
) -> Vec<SearchResult> {
    let weighted = result_sets.into_iter().map(|s| (s, 1.0)).collect();
    weighted_reciprocal_rank_fusion(weighted, k, limit)
}

/// Weighted Reciprocal Rank Fusion — each result set carries a weight that
/// scales its RRF contribution.
///
/// This is an extension of standard RRF where each source's contribution is
/// multiplied by its weight. A weight of 1.0 is neutral; higher weights give
/// that source more influence on the final ranking.
///
/// # Arguments
/// * `result_sets` - Pairs of (ranked result list, weight) from different search methods.
/// * `k` - Smoothing constant (typically 60).
/// * `limit` - Maximum number of results to return.
pub fn weighted_reciprocal_rank_fusion(
    result_sets: Vec<(Vec<SearchResult>, f64)>,
    k: f64,
    limit: usize,
) -> Vec<SearchResult> {
    if result_sets.is_empty() {
        return Vec::new();
    }

    let mut scores: HashMap<DocKey, f64> = HashMap::new();
    let mut best_result: HashMap<DocKey, SearchResult> = HashMap::new();

    for (result_set, weight) in &result_sets {
        for (rank, result) in result_set.iter().enumerate() {
            let key = DocKey {
                path: result.path.clone(),
                symbol: result.symbol.clone(),
                line: result.line,
            };

            let rrf_score = weight * (1.0 / (k + (rank + 1) as f64));
            *scores.entry(key.clone()).or_insert(0.0) += rrf_score;

            best_result
                .entry(key)
                .and_modify(|existing| {
                    if result.score > existing.score {
                        *existing = result.clone();
                    }
                })
                .or_insert_with(|| result.clone());
        }
    }

    let mut fused: Vec<(DocKey, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused.truncate(limit);

    fused
        .into_iter()
        .map(|(key, fused_score)| {
            let mut result = best_result.remove(&key).unwrap_or_else(|| SearchResult {
                path: key.path,
                symbol: key.symbol,
                snippet: String::new(),
                score: 0.0,
                source: crate::types::SearchSource::Keyword,
                line: key.line,
            });
            result.score = fused_score;
            result
        })
        .collect()
}

/// Key to identify a unique document across result sets.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct DocKey {
    path: PathBuf,
    symbol: Option<String>,
    line: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchResult, SearchSource};
    use std::path::PathBuf;

    fn make_result(path: &str, symbol: Option<&str>, score: f64, source: SearchSource) -> SearchResult {
        SearchResult {
            path: PathBuf::from(path),
            symbol: symbol.map(|s| s.to_string()),
            snippet: format!("snippet for {path}"),
            score,
            source,
            line: Some(1),
        }
    }

    #[test]
    fn test_rrf_single_set() {
        let set1 = vec![
            make_result("a.rs", None, 5.0, SearchSource::Keyword),
            make_result("b.rs", None, 3.0, SearchSource::Keyword),
            make_result("c.rs", None, 1.0, SearchSource::Keyword),
        ];

        let results = reciprocal_rank_fusion(vec![set1], 60.0, 10);
        assert_eq!(results.len(), 3);
        // First result should have highest RRF score: 1/(60+1)
        assert_eq!(results[0].path, PathBuf::from("a.rs"));
        assert_eq!(results[1].path, PathBuf::from("b.rs"));
        assert_eq!(results[2].path, PathBuf::from("c.rs"));
    }

    #[test]
    fn test_rrf_two_sets_boost_overlap() {
        // Document "a.rs" appears in both sets, should be boosted
        let set1 = vec![
            make_result("a.rs", None, 5.0, SearchSource::Keyword),
            make_result("b.rs", None, 3.0, SearchSource::Keyword),
        ];
        let set2 = vec![
            make_result("c.rs", None, 4.0, SearchSource::Semantic),
            make_result("a.rs", None, 2.0, SearchSource::Semantic),
        ];

        let results = reciprocal_rank_fusion(vec![set1, set2], 60.0, 10);
        assert_eq!(results.len(), 3);
        // "a.rs" gets 1/(60+1) + 1/(60+2) = boosted to top
        assert_eq!(results[0].path, PathBuf::from("a.rs"));
    }

    #[test]
    fn test_rrf_empty_input() {
        let results = reciprocal_rank_fusion(vec![], 60.0, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rrf_limit() {
        let set1 = vec![
            make_result("a.rs", None, 5.0, SearchSource::Keyword),
            make_result("b.rs", None, 4.0, SearchSource::Keyword),
            make_result("c.rs", None, 3.0, SearchSource::Keyword),
            make_result("d.rs", None, 2.0, SearchSource::Keyword),
        ];

        let results = reciprocal_rank_fusion(vec![set1], 60.0, 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_weighted_rrf_empty() {
        let results = super::weighted_reciprocal_rank_fusion(vec![], 60.0, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_weighted_rrf_single_set_unit_weight() {
        let set1 = vec![
            make_result("a.rs", None, 5.0, SearchSource::Keyword),
            make_result("b.rs", None, 3.0, SearchSource::Keyword),
        ];
        // With weight=1.0, should behave identically to plain RRF
        let weighted = super::weighted_reciprocal_rank_fusion(vec![(set1.clone(), 1.0)], 60.0, 10);
        let plain = reciprocal_rank_fusion(vec![set1], 60.0, 10);
        assert_eq!(weighted.len(), plain.len());
        assert_eq!(weighted[0].path, plain[0].path);
        assert!((weighted[0].score - plain[0].score).abs() < 1e-10);
    }

    #[test]
    fn test_weighted_rrf_high_weight_dominates() {
        // "a.rs" is #1 in a low-weight set, "b.rs" is #1 in a high-weight set.
        // "b.rs" should win overall.
        let set_low = vec![
            make_result("a.rs", None, 5.0, SearchSource::Keyword),
            make_result("b.rs", None, 3.0, SearchSource::Keyword),
        ];
        let set_high = vec![
            make_result("b.rs", None, 4.0, SearchSource::Semantic),
            make_result("a.rs", None, 2.0, SearchSource::Semantic),
        ];

        // Give set_high a much larger weight
        let results = super::weighted_reciprocal_rank_fusion(
            vec![(set_low, 0.1), (set_high, 10.0)],
            60.0,
            10,
        );
        assert_eq!(results[0].path, PathBuf::from("b.rs"));
    }

    #[test]
    fn test_weighted_rrf_limit() {
        let set1 = vec![
            make_result("a.rs", None, 5.0, SearchSource::Keyword),
            make_result("b.rs", None, 4.0, SearchSource::Keyword),
            make_result("c.rs", None, 3.0, SearchSource::Keyword),
        ];
        let results = super::weighted_reciprocal_rank_fusion(vec![(set1, 1.0)], 60.0, 2);
        assert_eq!(results.len(), 2);
    }
}
