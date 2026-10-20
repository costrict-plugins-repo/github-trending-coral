use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{SearchResult, SearchSource};

/// Info needed to embed a document via an external provider.
#[derive(Debug, Clone)]
pub struct EmbeddingDocInfo {
    pub path: PathBuf,
    pub symbol: Option<String>,
    pub line: Option<u32>,
    pub snippet: String,
}

/// A document stored in the semantic index.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SemanticDocument {
    path: PathBuf,
    symbol: Option<String>,
    snippet: String,
    line: Option<u32>,
}

/// Placeholder semantic search using TF-IDF cosine similarity.
///
/// This will be replaced by a real embedding model later. For now it provides
/// a reasonable approximation of semantic similarity by tokenizing on whitespace,
/// camelCase boundaries, and snake_case separators, then computing TF-IDF vectors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticIndex {
    documents: Vec<SemanticDocument>,
    vocabulary: HashMap<String, usize>,
    idf: Vec<f64>,
    tfidf_matrix: Vec<Vec<f64>>,
    #[serde(skip)]
    built: bool,
}

impl SemanticIndex {
    /// Create an empty semantic index.
    pub fn new() -> Self {
        Self {
            documents: Vec::new(),
            vocabulary: HashMap::new(),
            idf: Vec::new(),
            tfidf_matrix: Vec::new(),
            built: false,
        }
    }

    /// Add a document to the index. Call `build()` after adding all documents.
    pub fn add_document(
        &mut self,
        path: PathBuf,
        content: &str,
        symbol: Option<String>,
        line: Option<u32>,
    ) {
        let snippet = truncate_content(content, 200);
        self.documents.push(SemanticDocument {
            path,
            symbol,
            snippet,
            line,
        });
        // Mark as needing rebuild
        self.built = false;
    }

    /// Compute IDF and TF-IDF matrix from all added documents.
    pub fn build(&mut self) {
        if self.documents.is_empty() {
            self.built = true;
            return;
        }

        // Tokenize all documents
        let doc_tokens: Vec<Vec<String>> = self
            .documents
            .iter()
            .map(|d| tokenize(&d.snippet))
            .collect();

        // Build vocabulary from all tokens
        self.vocabulary.clear();
        for tokens in &doc_tokens {
            for token in tokens {
                let len = self.vocabulary.len();
                self.vocabulary.entry(token.clone()).or_insert(len);
            }
        }

        let vocab_size = self.vocabulary.len();
        let num_docs = self.documents.len();

        // Compute document frequency for each term
        let mut df = vec![0u32; vocab_size];
        for tokens in &doc_tokens {
            let mut seen = HashMap::new();
            for token in tokens {
                if let Some(&idx) = self.vocabulary.get(token) {
                    seen.entry(idx).or_insert(true);
                }
            }
            for &idx in seen.keys() {
                df[idx] += 1;
            }
        }

        // Compute IDF: log((N + 1) / (df + 1)) + 1 (smoothed)
        self.idf = df
            .iter()
            .map(|&d| ((num_docs as f64 + 1.0) / (d as f64 + 1.0)).ln() + 1.0)
            .collect();

        // Compute TF-IDF vectors for each document
        self.tfidf_matrix = Vec::with_capacity(num_docs);
        for tokens in &doc_tokens {
            let tf = compute_tf(tokens, &self.vocabulary, vocab_size);
            let tfidf: Vec<f64> = tf
                .iter()
                .zip(self.idf.iter())
                .map(|(&t, &i)| t * i)
                .collect();
            self.tfidf_matrix.push(tfidf);
        }

        self.built = true;
    }

    /// Search using TF-IDF cosine similarity against the query.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        if !self.built || self.documents.is_empty() || query.trim().is_empty() {
            return Vec::new();
        }

        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let vocab_size = self.vocabulary.len();
        let query_tf = compute_tf(&query_tokens, &self.vocabulary, vocab_size);

        // Compute query TF-IDF vector
        let query_tfidf: Vec<f64> = query_tf
            .iter()
            .zip(self.idf.iter())
            .map(|(&t, &i)| t * i)
            .collect();

        let query_norm = vector_norm(&query_tfidf);
        if query_norm == 0.0 {
            return Vec::new();
        }

        // Score each document by cosine similarity
        let mut scored: Vec<(usize, f64)> = self
            .tfidf_matrix
            .iter()
            .enumerate()
            .map(|(idx, doc_vec)| {
                let doc_norm = vector_norm(doc_vec);
                if doc_norm == 0.0 {
                    return (idx, 0.0);
                }
                let dot = dot_product(&query_tfidf, doc_vec);
                let cosine = dot / (query_norm * doc_norm);
                (idx, cosine)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        scored
            .into_iter()
            .map(|(idx, score)| {
                let doc = &self.documents[idx];
                SearchResult {
                    path: doc.path.clone(),
                    symbol: doc.symbol.clone(),
                    snippet: doc.snippet.clone(),
                    score,
                    source: SearchSource::Semantic,
                    line: doc.line,
                }
            })
            .collect()
    }

    /// Return document metadata for embedding — used by the engine to populate
    /// the vector store from an external `EmbeddingsProvider`.
    pub fn documents_for_embedding(&self) -> Vec<EmbeddingDocInfo> {
        self.documents
            .iter()
            .map(|d| EmbeddingDocInfo {
                path: d.path.clone(),
                symbol: d.symbol.clone(),
                line: d.line,
                snippet: d.snippet.clone(),
            })
            .collect()
    }

    /// Serialize and save the index to a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_vec(self).context("failed to serialize semantic index")?;
        std::fs::write(path, data)
            .with_context(|| format!("failed to write semantic index to {}", path.display()))?;
        Ok(())
    }

    /// Load a previously saved index.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .with_context(|| format!("failed to read semantic index from {}", path.display()))?;
        let mut index: Self =
            serde_json::from_slice(&data).context("failed to deserialize semantic index")?;
        index.built = true;
        Ok(index)
    }
}

impl Default for SemanticIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokenize text by splitting on whitespace, camelCase, and snake_case boundaries.
/// Produces lowercase tokens.
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();

    // First split on whitespace and punctuation
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '.' | ':' | '/' | '\\' | '"' | '\'' | '`' | '#' | '=' | '+' | '-' | '*' | '&' | '|' | '<' | '>' | '!' | '?' | '@'))
    {
        if word.is_empty() {
            continue;
        }

        // Split on snake_case
        for part in word.split('_') {
            if part.is_empty() {
                continue;
            }
            // Split on camelCase boundaries
            let camel_parts = split_camel_case(part);
            for p in camel_parts {
                let lower = p.to_lowercase();
                if lower.len() >= 2 {
                    tokens.push(lower);
                }
            }
        }
    }

    tokens
}

/// Split a string on camelCase boundaries.
fn split_camel_case(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut last_idx = 0;
    let bytes = s.as_bytes();

    for i in 1..bytes.len() {
        let prev_lower = (bytes[i - 1] as char).is_lowercase();
        let curr_upper = (bytes[i] as char).is_uppercase();

        if prev_lower && curr_upper {
            parts.push(&s[last_idx..i]);
            last_idx = i;
        }
    }

    parts.push(&s[last_idx..]);
    parts
}

/// Compute term frequency vector for the given tokens.
fn compute_tf(tokens: &[String], vocabulary: &HashMap<String, usize>, vocab_size: usize) -> Vec<f64> {
    let mut tf = vec![0.0f64; vocab_size];
    let total = tokens.len() as f64;
    if total == 0.0 {
        return tf;
    }

    for token in tokens {
        if let Some(&idx) = vocabulary.get(token) {
            tf[idx] += 1.0;
        }
    }

    // Normalize by document length
    for val in &mut tf {
        *val /= total;
    }

    tf
}

/// Compute the L2 norm of a vector.
fn vector_norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

/// Compute the dot product of two vectors.
fn dot_product(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Truncate content to a maximum character count at a word boundary.
fn truncate_content(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    let safe_end = text.floor_char_boundary(max_len);
    match text[..safe_end].rfind(char::is_whitespace) {
        Some(pos) => text[..pos].to_string(),
        None => text[..safe_end].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_camel_case() {
        let tokens = tokenize("authenticateUser validateJwtToken");
        assert!(tokens.contains(&"authenticate".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"validate".to_string()));
        assert!(tokens.contains(&"jwt".to_string()));
        assert!(tokens.contains(&"token".to_string()));
    }

    #[test]
    fn test_tokenize_snake_case() {
        let tokens = tokenize("connect_database create_pool");
        assert!(tokens.contains(&"connect".to_string()));
        assert!(tokens.contains(&"database".to_string()));
        assert!(tokens.contains(&"create".to_string()));
        assert!(tokens.contains(&"pool".to_string()));
    }

    #[test]
    fn test_semantic_search_basic() {
        let mut idx = SemanticIndex::new();
        idx.add_document(
            PathBuf::from("src/auth.rs"),
            "authenticate user validate jwt token session management",
            Some("authenticate_user".to_string()),
            Some(10),
        );
        idx.add_document(
            PathBuf::from("src/db.rs"),
            "connect database pool connection string postgres",
            Some("connect_database".to_string()),
            Some(20),
        );
        idx.add_document(
            PathBuf::from("src/http.rs"),
            "http server request response handler routing middleware",
            Some("start_server".to_string()),
            Some(1),
        );
        idx.build();

        let results = idx.search("authentication token validation", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].path, PathBuf::from("src/auth.rs"));
        assert_eq!(results[0].source, SearchSource::Semantic);
    }

    #[test]
    fn test_semantic_empty_query() {
        let mut idx = SemanticIndex::new();
        idx.add_document(PathBuf::from("test.rs"), "some content", None, None);
        idx.build();

        let results = idx.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_semantic_save_load() {
        let mut idx = SemanticIndex::new();
        idx.add_document(
            PathBuf::from("src/main.rs"),
            "main entry point application startup",
            Some("main".to_string()),
            Some(1),
        );
        idx.build();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic.json");
        idx.save(&path).unwrap();

        let loaded = SemanticIndex::load(&path).unwrap();
        let results = loaded.search("application entry", 5);
        assert!(!results.is_empty());
        assert_eq!(results[0].path, PathBuf::from("src/main.rs"));
    }
}
