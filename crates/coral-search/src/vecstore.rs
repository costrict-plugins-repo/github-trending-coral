//! Flat vector store -- cosine similarity search over f32 embeddings.
//!
//! Stores document vectors alongside metadata (path, symbol, line, snippet).
//! Search is brute-force cosine similarity against all entries (no approximate
//! nearest-neighbor index). This is sufficient for Coral's typical index sizes
//! (hundreds to low thousands of documents). An HNSW layer can be added later
//! if linear scan becomes a bottleneck.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single entry in the vector store: a document chunk with its embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEntry {
    pub path: PathBuf,
    pub symbol: Option<String>,
    pub line: Option<u32>,
    pub snippet: String,
    pub vector: Vec<f32>,
}

/// Flat vector store backed by JSON serialization.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct VectorStore {
    /// Dimensionality of stored vectors. All entries must match.
    pub dim: usize,
    /// The stored entries.
    pub entries: Vec<VectorEntry>,
    /// Model name used to generate the embeddings (for cache invalidation).
    pub model: String,
}

impl VectorStore {
    /// Create a new empty vector store for the given dimensionality and model.
    pub fn new(dim: usize, model: String) -> Self {
        Self {
            dim,
            entries: Vec::new(),
            model,
        }
    }

    /// Add an entry to the store. Returns an error if vector dimension mismatches.
    pub fn add(&mut self, entry: VectorEntry) -> anyhow::Result<()> {
        anyhow::ensure!(
            entry.vector.len() == self.dim,
            "vector dimension mismatch: expected {}, got {}",
            self.dim,
            entry.vector.len()
        );
        self.entries.push(entry);
        Ok(())
    }

    /// Search for the top-k most similar vectors to `query_vec`.
    ///
    /// Returns `(index, cosine_similarity)` pairs sorted by descending similarity.
    pub fn search(&self, query_vec: &[f32], limit: usize) -> Vec<(usize, f64)> {
        if self.entries.is_empty() || query_vec.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f64)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                let sim = cosine_similarity(query_vec, &entry.vector);
                (idx, sim)
            })
            .filter(|(_, sim)| *sim > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Persist the store to a JSON file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let data = serde_json::to_vec(self)
            .map_err(|e| anyhow::anyhow!("failed to serialize vector store: {e}"))?;
        std::fs::write(path, data)
            .map_err(|e| anyhow::anyhow!("failed to write vector store to {}: {e}", path.display()))?;
        Ok(())
    }

    /// Load a previously saved store from a JSON file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("failed to read vector store from {}: {e}", path.display()))?;
        let store: Self = serde_json::from_slice(&data)
            .map_err(|e| anyhow::anyhow!("failed to deserialize vector store: {e}"))?;
        Ok(store)
    }

    /// Number of entries in the store.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Cosine similarity between two f32 vectors.
///
/// Returns 0.0 if either vector has zero norm.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let mut dot: f64 = 0.0;
    let mut norm_a: f64 = 0.0;
    let mut norm_b: f64 = 0.0;

    for (x, y) in a.iter().zip(b.iter()) {
        let xf = *x as f64;
        let yf = *y as f64;
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0f32, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-10, "identical vectors should have similarity 1.0");
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-10, "orthogonal vectors should have similarity 0.0");
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![-1.0f32, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-10, "opposite vectors should have similarity -1.0");
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![0.0f32, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0, "zero vector should give 0.0 similarity");
    }

    #[test]
    fn test_vecstore_add_and_search() {
        let mut store = VectorStore::new(3, "test-model".to_string());

        store.add(VectorEntry {
            path: PathBuf::from("a.rs"),
            symbol: Some("foo".to_string()),
            line: Some(1),
            snippet: "fn foo()".to_string(),
            vector: vec![1.0, 0.0, 0.0],
        }).unwrap();
        store.add(VectorEntry {
            path: PathBuf::from("b.rs"),
            symbol: Some("bar".to_string()),
            line: Some(5),
            snippet: "fn bar()".to_string(),
            vector: vec![0.9, 0.1, 0.0],
        }).unwrap();
        store.add(VectorEntry {
            path: PathBuf::from("c.rs"),
            symbol: Some("baz".to_string()),
            line: Some(10),
            snippet: "fn baz()".to_string(),
            vector: vec![0.0, 0.0, 1.0],
        }).unwrap();

        // Query close to a.rs and b.rs
        let results = store.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        // a.rs should be first (exact match)
        assert_eq!(results[0].0, 0);
        // b.rs should be second (close)
        assert_eq!(results[1].0, 1);
    }

    #[test]
    fn test_vecstore_search_empty() {
        let store = VectorStore::new(3, "test".to_string());
        let results = store.search(&[1.0, 0.0, 0.0], 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_vecstore_search_limit() {
        let mut store = VectorStore::new(3, "test".to_string());
        for i in 0..10 {
            store.add(VectorEntry {
                path: PathBuf::from(format!("{i}.rs")),
                symbol: None,
                line: None,
                snippet: String::new(),
                vector: vec![1.0, (i as f32) * 0.01, 0.0],
            }).unwrap();
        }

        let results = store.search(&[1.0, 0.0, 0.0], 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_vecstore_save_load() {
        let mut store = VectorStore::new(3, "test-model".to_string());
        store.add(VectorEntry {
            path: PathBuf::from("src/main.rs"),
            symbol: Some("main".to_string()),
            line: Some(1),
            snippet: "fn main()".to_string(),
            vector: vec![0.5, 0.5, 0.0],
        }).unwrap();
        store.add(VectorEntry {
            path: PathBuf::from("src/lib.rs"),
            symbol: None,
            line: None,
            snippet: "mod foo;".to_string(),
            vector: vec![0.0, 1.0, 0.0],
        }).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.json");
        store.save(&path).unwrap();

        let loaded = VectorStore::load(&path).unwrap();
        assert_eq!(loaded.dim, 3);
        assert_eq!(loaded.model, "test-model");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.entries[0].path, PathBuf::from("src/main.rs"));
        assert_eq!(loaded.entries[0].vector, vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn test_vecstore_len_and_is_empty() {
        let mut store = VectorStore::new(2, "m".to_string());
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        store.add(VectorEntry {
            path: PathBuf::from("x.rs"),
            symbol: None,
            line: None,
            snippet: String::new(),
            vector: vec![1.0, 0.0],
        }).unwrap();
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
    }
}
