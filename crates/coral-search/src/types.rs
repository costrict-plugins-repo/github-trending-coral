use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single search result returned by any search source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: PathBuf,
    pub symbol: Option<String>,
    /// Relevant code or text snippet.
    pub snippet: String,
    pub score: f64,
    pub source: SearchSource,
    pub line: Option<u32>,
}

/// Which search method produced the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchSource {
    /// BM25 full-text match.
    Keyword,
    /// Embedding / vector similarity.
    Semantic,
    /// Graph-based structural match.
    Structural,
}

/// A query to the hybrid search engine.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// The natural-language or keyword query text.
    pub text: String,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Which search paths to use (keyword, semantic, structural).
    pub sources: Vec<SearchSource>,
    /// Optional glob pattern to restrict file paths.
    pub file_filter: Option<String>,
}

/// The type of document being indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocType {
    /// Source code file or symbol.
    Code,
    /// Wiki page (`.wiki/*.md`).
    Wiki,
}
