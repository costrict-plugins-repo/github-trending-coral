//! Trigram-based fuzzy symbol search.
//!
//! Matches symbol names against a query using trigram overlap scoring
//! (Jaccard similarity). This provides fast approximate matching without
//! requiring the `fst` crate or full-text indexes.

use std::collections::HashSet;

/// A trigram index for fast fuzzy matching over symbol names.
#[derive(Debug, Clone, Default)]
pub struct TrigramIndex {
    entries: Vec<TrigramEntry>,
}

#[derive(Debug, Clone)]
struct TrigramEntry {
    pub name: String,
    pub file: String,
    pub line: u32,
    trigrams: HashSet<String>,
}

/// A single fuzzy match result.
#[derive(Debug, Clone)]
pub struct FuzzyMatch {
    /// The matched symbol name.
    pub name: String,
    /// The file containing the symbol.
    pub file: String,
    /// The line number of the symbol.
    pub line: u32,
    /// Jaccard similarity score (0.0 to 1.0).
    pub score: f64,
}

impl TrigramIndex {
    /// Create a new empty trigram index.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a symbol entry to the index.
    pub fn add(&mut self, name: &str, file: &str, line: u32) {
        let trigrams = extract_trigrams(name);
        self.entries.push(TrigramEntry {
            name: name.to_string(),
            file: file.to_string(),
            line,
            trigrams,
        });
    }

    /// Fuzzy search: find entries whose names have high trigram overlap with the query.
    /// Returns `FuzzyMatch` results sorted by score descending, limited to `limit` results.
    pub fn search(&self, query: &str, limit: usize) -> Vec<FuzzyMatch> {
        let query_trigrams = extract_trigrams(query);
        if query_trigrams.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<FuzzyMatch> = self
            .entries
            .iter()
            .map(|entry| {
                let overlap = entry.trigrams.intersection(&query_trigrams).count();
                let union = entry.trigrams.union(&query_trigrams).count();
                let score = if union == 0 {
                    0.0
                } else {
                    overlap as f64 / union as f64
                }; // Jaccard similarity
                FuzzyMatch {
                    name: entry.name.clone(),
                    file: entry.file.clone(),
                    line: entry.line,
                    score,
                }
            })
            .filter(|m| m.score > 0.0)
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        scored
    }

    /// Return the number of entries in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Extract character trigrams from a string (lowercased, with padding).
fn extract_trigrams(s: &str) -> HashSet<String> {
    let s = s.to_lowercase();
    let padded = format!("  {s} "); // pad with spaces for edge trigrams
    padded
        .as_bytes()
        .windows(3)
        .map(|w| String::from_utf8_lossy(w).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_trigrams_basic() {
        let trigrams = extract_trigrams("foo");
        // "  foo " -> "  f", " fo", "foo", "oo "
        assert!(trigrams.contains("  f"));
        assert!(trigrams.contains(" fo"));
        assert!(trigrams.contains("foo"));
        assert!(trigrams.contains("oo "));
        assert_eq!(trigrams.len(), 4);
    }

    #[test]
    fn test_extract_trigrams_case_insensitive() {
        let lower = extract_trigrams("Foo");
        let upper = extract_trigrams("foo");
        assert_eq!(lower, upper);
    }

    #[test]
    fn test_extract_trigrams_short_string() {
        // Single char: "  a " -> "  a", " a "
        let trigrams = extract_trigrams("a");
        assert_eq!(trigrams.len(), 2);
    }

    #[test]
    fn test_extract_trigrams_empty_string() {
        let trigrams = extract_trigrams("");
        // "   " -> one trigram "   "
        assert_eq!(trigrams.len(), 1);
    }

    #[test]
    fn test_fuzzy_exact_match_scores_highest() {
        let mut index = TrigramIndex::new();
        index.add("authenticate_user", "src/auth.rs", 10);
        index.add("create_user", "src/users.rs", 20);
        index.add("delete_session", "src/session.rs", 30);

        let results = index.search("authenticate_user", 10);
        assert!(!results.is_empty());
        // Exact match should have score 1.0 (identical trigram sets)
        assert!((results[0].score - 1.0).abs() < f64::EPSILON);
        assert_eq!(results[0].name, "authenticate_user");
    }

    #[test]
    fn test_fuzzy_partial_match() {
        let mut index = TrigramIndex::new();
        index.add("authenticate_user", "src/auth.rs", 10);
        index.add("create_user", "src/users.rs", 20);
        index.add("delete_session", "src/session.rs", 30);

        let results = index.search("auth_user", 10);
        assert!(!results.is_empty());
        // "authenticate_user" should score highest because it shares more trigrams
        assert_eq!(results[0].name, "authenticate_user");
        assert!(results[0].score > 0.0);
        assert!(results[0].score < 1.0);
    }

    #[test]
    fn test_fuzzy_no_match() {
        let mut index = TrigramIndex::new();
        index.add("foo", "src/foo.rs", 1);
        index.add("bar", "src/bar.rs", 2);

        // "xyz" shares no trigrams with "foo" or "bar"
        let results = index.search("xyz", 10);
        // Might still get some weak matches from space-padding overlap
        // but truly unrelated strings should have very low scores
        for result in &results {
            assert!(result.score < 0.5);
        }
    }

    #[test]
    fn test_fuzzy_empty_query() {
        let mut index = TrigramIndex::new();
        index.add("foo", "src/foo.rs", 1);

        let results = index.search("", 10);
        // Empty-string trigrams only contain "   " which may weakly match
        // but the implementation filters by score > 0.0
        // This is acceptable behavior
        assert!(results.len() <= 1);
    }

    #[test]
    fn test_fuzzy_limit() {
        let mut index = TrigramIndex::new();
        for i in 0..100 {
            index.add(&format!("symbol_{i}"), "src/lib.rs", i);
        }

        let results = index.search("symbol", 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_index_len_and_is_empty() {
        let mut index = TrigramIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);

        index.add("foo", "src/foo.rs", 1);
        assert!(!index.is_empty());
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_fuzzy_case_insensitive_search() {
        let mut index = TrigramIndex::new();
        index.add("HttpServer", "src/server.rs", 5);

        let results = index.search("httpserver", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "HttpServer");
        assert!((results[0].score - 1.0).abs() < f64::EPSILON);
    }
}
